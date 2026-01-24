//! AI Contexters
//!
//! Extracts timeline and decisions from AI agent session files:
//! - Claude Code: ~/.claude/projects/*/*.jsonl
//! - Codex: ~/.codex/history.jsonl
//! - Gemini: ~/.gemini/tmp/<hash>/chats/session-*.json
//!
//! Features: incremental extraction, deduplication, rotation, append mode.
//!
//! Created by M&K (c)2026 VetCoders

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use clap::{Parser, Subcommand};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ai_contexters::chunker::{self, ChunkerConfig};
use ai_contexters::memex::{self, MemexConfig};
use ai_contexters::output::{self, OutputConfig, OutputFormat, OutputMode, ReportMetadata};
use ai_contexters::sources::{self, ExtractionConfig};
use ai_contexters::state::StateManager;
use ai_contexters::store;

/// Calculate cutoff time based on today/yesterday flags or hours
/// Returns (cutoff_start, cutoff_end_option, period_label)
fn calc_cutoff(hours: u64, today: bool, yesterday: bool) -> Result<(DateTime<Utc>, Option<DateTime<Utc>>, String)> {
    if today {
        let now = Local::now();
        let start_of_today = now.date_naive()
            .and_hms_opt(0, 0, 0)
            .context("Failed to create start of today timestamp")?;
        let cutoff = Local.from_local_datetime(&start_of_today)
            .single()
            .context("Ambiguous or invalid local time (DST transition?)")?
            .with_timezone(&Utc);
        Ok((cutoff, None, "today".to_string()))
    } else if yesterday {
        let now = Local::now();
        let yesterday_date = now.date_naive() - Duration::days(1);
        let start_of_yesterday = yesterday_date
            .and_hms_opt(0, 0, 0)
            .context("Failed to create start of yesterday timestamp")?;
        let end_of_yesterday = yesterday_date
            .and_hms_opt(23, 59, 59)
            .context("Failed to create end of yesterday timestamp")?;
        let cutoff_start = Local.from_local_datetime(&start_of_yesterday)
            .single()
            .context("Ambiguous or invalid local time (DST transition?)")?
            .with_timezone(&Utc);
        let cutoff_end = Local.from_local_datetime(&end_of_yesterday)
            .single()
            .context("Ambiguous or invalid local time (DST transition?)")?
            .with_timezone(&Utc);
        Ok((cutoff_start, Some(cutoff_end), "yesterday".to_string()))
    } else {
        let cutoff = Utc::now() - Duration::hours(hours as i64);
        Ok((cutoff, None, format!("last {} hours", hours)))
    }
}

/// AI Contexters - timeline and decisions from AI sessions
#[derive(Parser)]
#[command(name = "ai-contexters")]
#[command(author = "M&K (c)2026 VetCoders")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Extract timeline from Claude Code sessions
    Claude {
        /// Project directory filter (e.g., "CodeScribe" or full path)
        #[arg(short, long)]
        project: Option<String>,

        /// Hours to look back (default: 48)
        #[arg(short = 'H', long, default_value = "48")]
        hours: u64,

        /// Show only today's entries
        #[arg(long, conflicts_with_all = ["hours", "yesterday"])]
        today: bool,

        /// Show only yesterday's entries
        #[arg(long, conflicts_with_all = ["hours", "today"])]
        yesterday: bool,

        /// Output directory
        #[arg(short, long, default_value = ".")]
        output: PathBuf,

        /// Output format: md, json, both
        #[arg(short, long, default_value = "both")]
        format: String,

        /// Search pattern (regex) - filter messages
        #[arg(short = 'g', long)]
        grep: Option<String>,

        /// Append to a single timeline file instead of creating new files
        #[arg(long)]
        append_to: Option<PathBuf>,

        /// Keep only last N output files (0 = unlimited)
        #[arg(long, default_value = "0")]
        rotate: usize,

        /// Use incremental mode (skip already-processed entries)
        #[arg(long)]
        incremental: bool,

        /// Include assistant messages (can be large)
        #[arg(long)]
        include_assistant: bool,

        /// Include loctree snapshot in output
        #[arg(long)]
        loctree: bool,

        /// Project root for loctree snapshot (defaults to cwd)
        #[arg(long)]
        project_root: Option<PathBuf>,

        /// Also chunk and sync to memex after extraction
        #[arg(long)]
        memex: bool,
    },

    /// Extract timeline from Codex history
    Codex {
        /// Project/repo filter
        #[arg(short, long)]
        project: Option<String>,

        /// Hours to look back (default: 48)
        #[arg(short = 'H', long, default_value = "48")]
        hours: u64,

        /// Show only today's entries
        #[arg(long, conflicts_with_all = ["hours", "yesterday"])]
        today: bool,

        /// Show only yesterday's entries
        #[arg(long, conflicts_with_all = ["hours", "today"])]
        yesterday: bool,

        /// Output directory
        #[arg(short, long, default_value = ".")]
        output: PathBuf,

        /// Output format: md, json, both
        #[arg(short, long, default_value = "both")]
        format: String,

        /// Search pattern (regex) - filter messages
        #[arg(short = 'g', long)]
        grep: Option<String>,

        /// Append to a single timeline file
        #[arg(long)]
        append_to: Option<PathBuf>,

        /// Keep only last N output files (0 = unlimited)
        #[arg(long, default_value = "0")]
        rotate: usize,

        /// Use incremental mode
        #[arg(long)]
        incremental: bool,

        /// Include loctree snapshot
        #[arg(long)]
        loctree: bool,

        /// Project root for loctree snapshot
        #[arg(long)]
        project_root: Option<PathBuf>,

        /// Also chunk and sync to memex after extraction
        #[arg(long)]
        memex: bool,
    },

    /// Extract from all agents (Claude + Codex + Gemini)
    All {
        /// Project filter
        #[arg(short, long)]
        project: Option<String>,

        /// Hours to look back
        #[arg(short = 'H', long, default_value = "48")]
        hours: u64,

        /// Show only today's entries
        #[arg(long, conflicts_with_all = ["hours", "yesterday"])]
        today: bool,

        /// Show only yesterday's entries
        #[arg(long, conflicts_with_all = ["hours", "today"])]
        yesterday: bool,

        /// Output directory
        #[arg(short, long, default_value = ".")]
        output: PathBuf,

        /// Search pattern (regex) - filter messages
        #[arg(short = 'g', long)]
        grep: Option<String>,

        /// Append to a single timeline file
        #[arg(long)]
        append_to: Option<PathBuf>,

        /// Keep only last N output files (0 = unlimited)
        #[arg(long, default_value = "0")]
        rotate: usize,

        /// Use incremental mode
        #[arg(long)]
        incremental: bool,

        /// Include assistant messages
        #[arg(long)]
        include_assistant: bool,

        /// Include loctree snapshot
        #[arg(long)]
        loctree: bool,

        /// Project root for loctree snapshot
        #[arg(long)]
        project_root: Option<PathBuf>,

        /// Also chunk and sync to memex after extraction
        #[arg(long)]
        memex: bool,
    },

    /// Show statistics dashboard
    Stats {
        /// Agent type: claude, codex, all
        #[arg(short, long, default_value = "all")]
        agent: String,

        /// Hours to look back (default: 168 = 7 days)
        #[arg(short = 'H', long, default_value = "168")]
        hours: u64,

        /// Show only today's entries
        #[arg(long, conflicts_with_all = ["hours", "yesterday"])]
        today: bool,

        /// Show only yesterday's entries
        #[arg(long, conflicts_with_all = ["hours", "today"])]
        yesterday: bool,

        /// Project filter
        #[arg(short, long)]
        project: Option<String>,
    },

    /// Store contexts in central store (~/.ai-contexters/) and optionally sync to memex
    Store {
        /// Project name (required for store organization)
        #[arg(short, long)]
        project: Option<String>,

        /// Agent filter: claude, codex, gemini (default: all)
        #[arg(short, long)]
        agent: Option<String>,

        /// Hours to look back (default: 48)
        #[arg(short = 'H', long, default_value = "48")]
        hours: u64,

        /// Include assistant messages
        #[arg(long)]
        include_assistant: bool,

        /// Also chunk and sync to memex
        #[arg(long)]
        memex: bool,
    },

    /// Sync stored chunks to rmcp-memex vector memory
    MemexSync {
        /// Namespace in vector store
        #[arg(short, long, default_value = "ai-contexts")]
        namespace: String,

        /// Use per-chunk upsert instead of batch index
        #[arg(long)]
        per_chunk: bool,

        /// Override LanceDB path
        #[arg(long)]
        db_path: Option<PathBuf>,
    },

    /// List available projects/sessions
    List,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ai_contexters=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Claude {
            project, hours, today, yesterday, output, format, grep, append_to,
            rotate, incremental, include_assistant, loctree, project_root, memex,
        } => {
            let (cutoff, cutoff_end, _period) = calc_cutoff(hours, today, yesterday)?;
            run_extraction(
                &["claude"],
                project, cutoff, cutoff_end, &output, &format, grep, append_to,
                rotate, incremental, include_assistant, loctree, project_root, memex,
            )?;
        }
        Commands::Codex {
            project, hours, today, yesterday, output, format, grep, append_to,
            rotate, incremental, loctree, project_root, memex,
        } => {
            let (cutoff, cutoff_end, _period) = calc_cutoff(hours, today, yesterday)?;
            run_extraction(
                &["codex"],
                project, cutoff, cutoff_end, &output, &format, grep, append_to,
                rotate, incremental, false, loctree, project_root, memex,
            )?;
        }
        Commands::All {
            project, hours, today, yesterday, output, grep, append_to,
            rotate, incremental, include_assistant, loctree, project_root, memex,
        } => {
            let (cutoff, cutoff_end, _period) = calc_cutoff(hours, today, yesterday)?;
            run_extraction(
                &["claude", "codex", "gemini"],
                project, cutoff, cutoff_end, &output, "both", grep, append_to,
                rotate, incremental, include_assistant, loctree, project_root, memex,
            )?;
        }
        Commands::Stats {
            agent, hours, today, yesterday, project,
        } => {
            let (cutoff, cutoff_end, period) = calc_cutoff(hours, today, yesterday)?;
            show_stats(&agent, cutoff, cutoff_end, &period, project)?;
        }
        Commands::Store {
            project, agent, hours, include_assistant, memex,
        } => {
            run_store(project, agent, hours, include_assistant, memex)?;
        }
        Commands::MemexSync {
            namespace, per_chunk, db_path,
        } => {
            run_memex_sync(&namespace, per_chunk, db_path)?;
        }
        Commands::List => {
            let sources = sources::list_available_sources()?;
            if sources.is_empty() {
                println!("No AI agent session sources found.");
            } else {
                println!("=== Available Sources ===\n");
                for info in &sources {
                    let size_mb = info.size_bytes as f64 / 1024.0 / 1024.0;
                    println!(
                        "  [{:>7}] {} ({} sessions, {:.1} MB)",
                        info.agent,
                        info.path.display(),
                        info.sessions,
                        size_mb,
                    );
                }
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_extraction(
    agents: &[&str],
    project: Option<String>,
    cutoff: DateTime<Utc>,
    cutoff_end: Option<DateTime<Utc>>,
    output_dir: &Path,
    format: &str,
    grep_pattern: Option<String>,
    append_to: Option<PathBuf>,
    rotate: usize,
    incremental: bool,
    include_assistant: bool,
    include_loctree: bool,
    project_root: Option<PathBuf>,
    sync_memex: bool,
) -> Result<()> {
    // Compile grep regex if provided
    let grep_re = grep_pattern
        .as_ref()
        .map(|p| Regex::new(&format!("(?i){}", p)))
        .transpose()
        .context("Invalid grep regex")?;

    // Load state for incremental/dedup
    let mut state = StateManager::load();

    // Determine watermark (incremental mode uses per-source watermark)
    let watermark = if incremental {
        let source_key = format!(
            "{}:{}",
            agents.join("+"),
            project.as_deref().unwrap_or("all")
        );
        state.get_watermark(&source_key)
    } else {
        None
    };

    let config = ExtractionConfig {
        project_filter: project.clone(),
        cutoff,
        include_assistant,
        watermark,
    };

    // Extract from requested sources
    let mut entries = Vec::new();

    for &agent in agents {
        let agent_entries = match agent {
            "claude" => sources::extract_claude(&config)?,
            "codex" => sources::extract_codex(&config)?,
            "gemini" => sources::extract_gemini(&config)?,
            _ => Vec::new(),
        };

        eprintln!("  [{}] {} entries", agent, agent_entries.len());
        entries.extend(agent_entries);
    }

    // Dedup via state
    let pre_dedup = entries.len();
    entries.retain(|e| {
        let hash = StateManager::content_hash(&e.agent, &e.session_id, e.timestamp.timestamp(), &e.message);
        state.is_new(hash)
    });

    if pre_dedup != entries.len() {
        eprintln!(
            "  Dedup: {} → {} entries (skipped {} seen)",
            pre_dedup,
            entries.len(),
            pre_dedup - entries.len(),
        );
    }

    // Filter by cutoff_end (for --yesterday)
    if let Some(end) = cutoff_end {
        entries.retain(|e| e.timestamp <= end);
    }

    // Filter by grep pattern
    if let Some(ref re) = grep_re {
        let pre_grep = entries.len();
        entries.retain(|e| re.is_match(&e.message));
        if pre_grep != entries.len() {
            eprintln!("  Grep: {} → {} entries", pre_grep, entries.len());
        }
    }

    // Sort by timestamp
    entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    // Convert sources::TimelineEntry → output::TimelineEntry
    let output_entries: Vec<output::TimelineEntry> = entries
        .iter()
        .map(|e| output::TimelineEntry {
            timestamp: e.timestamp,
            agent: e.agent.clone(),
            session_id: e.session_id.clone(),
            role: e.role.clone(),
            message: e.message.clone(),
            branch: e.branch.clone(),
            cwd: e.cwd.clone(),
        })
        .collect();

    // Collect unique sessions
    let mut sessions: Vec<String> = entries.iter().map(|e| e.session_id.clone()).collect();
    sessions.sort();
    sessions.dedup();

    // Calculate hours_back from cutoff for metadata
    let hours_back = (Utc::now() - cutoff).num_hours() as u64;

    let metadata = ReportMetadata {
        generated_at: Utc::now(),
        project_filter: project.clone(),
        hours_back,
        total_entries: output_entries.len(),
        sessions: sessions.clone(),
    };

    // Build output config
    let out_format = match format {
        "md" => OutputFormat::Markdown,
        "json" => OutputFormat::Json,
        _ => OutputFormat::Both,
    };

    let mode = if let Some(ref path) = append_to {
        OutputMode::AppendTimeline(path.clone())
    } else {
        OutputMode::NewFile
    };

    let out_config = OutputConfig {
        dir: output_dir.to_path_buf(),
        format: out_format,
        mode,
        max_files: rotate,
        max_message_chars: 0, // no truncation
        include_loctree,
        project_root,
    };

    // Write output
    let written = output::write_report(&out_config, &output_entries, &metadata)?;

    for path in &written {
        eprintln!("  → {}", path.display());
    }

    // Rotation
    if rotate > 0 {
        let prefix = agents.join("_");
        let deleted = output::rotate_outputs(output_dir, &prefix, rotate)?;
        if deleted > 0 {
            eprintln!("  Rotated: deleted {} old files", deleted);
        }
    }

    // Update state
    if !entries.is_empty() {
        for e in &entries {
            let hash = StateManager::content_hash(&e.agent, &e.session_id, e.timestamp.timestamp(), &e.message);
            state.mark_seen(hash);
        }

        if incremental {
            let source_key = format!(
                "{}:{}",
                agents.join("+"),
                project.as_deref().unwrap_or("all")
            );
            if let Some(latest) = entries.last() {
                state.update_watermark(&source_key, latest.timestamp);
            }
        }

        state.record_run(entries.len(), agents.iter().map(|s| s.to_string()).collect());
        state.prune_old_hashes(50_000);
        state.save()?;
    }

    eprintln!(
        "✓ {} entries from {} sessions ({})",
        output_entries.len(),
        sessions.len(),
        agents.join("+"),
    );

    // Memex sync: chunk entries and push to vector store
    if sync_memex && !output_entries.is_empty() {
        let proj_name = project.as_deref().unwrap_or("unknown");
        let agent_name = agents.join("+");

        let chunker_config = ChunkerConfig::default();
        let chunks = chunker::chunk_entries(&output_entries, proj_name, &agent_name, &chunker_config);

        if !chunks.is_empty() {
            let chunks_dir = store::chunks_dir()?;
            chunker::write_chunks_to_dir(&chunks, &chunks_dir)?;
            eprintln!("  Chunked: {} chunks ({}) → {}", chunks.len(), chunker::chunk_summary(&chunks), chunks_dir.display());

            let memex_config = MemexConfig::default();
            match memex::sync_new_chunks(&chunks_dir, &memex_config) {
                Ok(result) => {
                    eprintln!(
                        "  Memex: {} pushed, {} skipped",
                        result.chunks_pushed, result.chunks_skipped,
                    );
                    for err in &result.errors {
                        eprintln!("  Memex error: {}", err);
                    }
                }
                Err(e) => eprintln!("  Memex sync failed: {}", e),
            }
        }
    }

    Ok(())
}

/// Store extracted contexts in central store and optionally sync to memex.
fn run_store(
    project: Option<String>,
    agent: Option<String>,
    hours: u64,
    include_assistant: bool,
    sync_memex: bool,
) -> Result<()> {
    let cutoff = Utc::now() - chrono::Duration::hours(hours as i64);

    let agents: Vec<&str> = match agent.as_deref() {
        Some("claude") => vec!["claude"],
        Some("codex") => vec!["codex"],
        Some("gemini") => vec!["gemini"],
        _ => vec!["claude", "codex", "gemini"],
    };

    let config = ExtractionConfig {
        project_filter: project.clone(),
        cutoff,
        include_assistant,
        watermark: None,
    };

    let mut all_entries = Vec::new();
    for &ag in &agents {
        let agent_entries = match ag {
            "claude" => sources::extract_claude(&config)?,
            "codex" => sources::extract_codex(&config)?,
            "gemini" => sources::extract_gemini(&config)?,
            _ => Vec::new(),
        };
        eprintln!("  [{}] {} entries", ag, agent_entries.len());
        all_entries.extend(agent_entries);
    }

    all_entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    if all_entries.is_empty() {
        eprintln!("No entries found.");
        return Ok(());
    }

    // Convert to output::TimelineEntry
    let output_entries: Vec<output::TimelineEntry> = all_entries
        .iter()
        .map(|e| output::TimelineEntry {
            timestamp: e.timestamp,
            agent: e.agent.clone(),
            session_id: e.session_id.clone(),
            role: e.role.clone(),
            message: e.message.clone(),
            branch: e.branch.clone(),
            cwd: e.cwd.clone(),
        })
        .collect();

    // Group by agent+date and write to central store
    let proj_name = project.as_deref().unwrap_or("unknown");
    let mut index = store::load_index();
    let mut stored_count = 0usize;

    // Group entries by (agent, date)
    let mut groups: std::collections::BTreeMap<(String, String), Vec<output::TimelineEntry>> =
        std::collections::BTreeMap::new();

    for entry in &output_entries {
        let date = entry.timestamp.format("%Y-%m-%d").to_string();
        groups
            .entry((entry.agent.clone(), date))
            .or_default()
            .push(entry.clone());
    }

    for ((agent_name, date), group_entries) in &groups {
        let path = store::write_context(proj_name, agent_name, date, group_entries)?;
        store::update_index(&mut index, proj_name, agent_name, date, group_entries.len());
        stored_count += group_entries.len();
        eprintln!("  → {} ({} entries)", path.display(), group_entries.len());
    }

    store::save_index(&index)?;
    eprintln!("✓ Stored {} entries in {} groups", stored_count, groups.len());

    // Chunk and sync to memex if requested
    if sync_memex && !output_entries.is_empty() {
        let agent_label = agents.join("+");
        let chunker_config = ChunkerConfig::default();
        let chunks = chunker::chunk_entries(&output_entries, proj_name, &agent_label, &chunker_config);

        if !chunks.is_empty() {
            let chunks_dir = store::chunks_dir()?;
            chunker::write_chunks_to_dir(&chunks, &chunks_dir)?;
            eprintln!("  Chunked: {}", chunker::chunk_summary(&chunks));

            let memex_config = MemexConfig::default();
            match memex::sync_new_chunks(&chunks_dir, &memex_config) {
                Ok(result) => {
                    eprintln!("  Memex: {} pushed, {} skipped", result.chunks_pushed, result.chunks_skipped);
                }
                Err(e) => eprintln!("  Memex sync failed: {}", e),
            }
        }
    }

    Ok(())
}

/// Sync stored chunks to rmcp-memex vector memory.
fn run_memex_sync(namespace: &str, per_chunk: bool, db_path: Option<PathBuf>) -> Result<()> {
    if !memex::check_memex_available() {
        eprintln!("Error: rmcp-memex not found in PATH.");
        eprintln!("Install with: cargo install rmcp-memex");
        std::process::exit(1);
    }

    let chunks_dir = store::chunks_dir()?;
    if !chunks_dir.exists() {
        eprintln!("No chunks directory found at: {}", chunks_dir.display());
        eprintln!("Run `ai-contexters store --memex` first to generate chunks.");
        return Ok(());
    }

    let config = MemexConfig {
        namespace: namespace.to_string(),
        db_path,
        batch_mode: !per_chunk,
    };

    eprintln!("Syncing chunks from: {}", chunks_dir.display());
    eprintln!("  Namespace: {}", config.namespace);
    eprintln!("  Mode: {}", if config.batch_mode { "batch" } else { "per-chunk" });

    let result = memex::sync_new_chunks(&chunks_dir, &config)?;

    eprintln!(
        "✓ Memex sync: {} pushed, {} skipped",
        result.chunks_pushed, result.chunks_skipped,
    );

    for err in &result.errors {
        eprintln!("  Error: {}", err);
    }

    Ok(())
}

/// Show statistics dashboard
fn show_stats(
    agent: &str,
    cutoff: DateTime<Utc>,
    cutoff_end: Option<DateTime<Utc>>,
    period: &str,
    project_filter: Option<String>,
) -> Result<()> {
    let config = sources::ExtractionConfig {
        project_filter: project_filter.clone(),
        cutoff,
        include_assistant: true,
        watermark: None,
    };

    let mut total_messages = 0usize;
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut sessions_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut by_date: HashMap<String, usize> = HashMap::new();
    let mut by_project: HashMap<String, usize> = HashMap::new();

    // Claude stats
    if agent == "claude" || agent == "all" {
        let entries = sources::extract_claude(&config).unwrap_or_default();
        for entry in &entries {
            // Filter by cutoff_end
            if let Some(end) = cutoff_end {
                if entry.timestamp > end {
                    continue;
                }
            }
            total_messages += 1;
            if entry.role == "user" {
                user_messages += 1;
            } else {
                assistant_messages += 1;
            }
            sessions_set.insert(entry.session_id.clone());

            let date = entry.timestamp.format("%Y-%m-%d").to_string();
            *by_date.entry(date).or_insert(0) += 1;

            // Extract project from cwd
            if let Some(ref cwd) = entry.cwd {
                let short: String = cwd.chars().rev().take(40).collect::<String>().chars().rev().collect();
                *by_project.entry(short).or_insert(0) += 1;
            }
        }
    }

    // Codex stats
    if agent == "codex" || agent == "all" {
        let entries = sources::extract_codex(&config).unwrap_or_default();
        for entry in &entries {
            if let Some(end) = cutoff_end {
                if entry.timestamp > end {
                    continue;
                }
            }
            total_messages += 1;
            user_messages += 1; // Codex only stores user messages
            sessions_set.insert(entry.session_id.clone());

            let date = entry.timestamp.format("%Y-%m-%d").to_string();
            *by_date.entry(date).or_insert(0) += 1;
            *by_project.entry("codex".to_string()).or_insert(0) += 1;
        }
    }

    // Print stats
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                    AI CONTEXTERS STATS                       ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Period: {:52} ║", period);
    if let Some(ref p) = project_filter {
        println!("║  Filter: {:52} ║", p);
    }
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  TOTALS                                                      ║");
    println!("║     Sessions:    {:6}                                      ║", sessions_set.len());
    println!("║     Messages:    {:6}                                      ║", total_messages);
    println!("║       User:      {:6}                                      ║", user_messages);
    println!("║       Agent:     {:6}                                      ║", assistant_messages);
    println!("╠══════════════════════════════════════════════════════════════╣");

    // Messages per day
    if !by_date.is_empty() {
        println!("║  ACTIVITY BY DAY                                             ║");
        let mut dates: Vec<_> = by_date.iter().collect();
        dates.sort_by(|a, b| a.0.cmp(b.0));
        for (date, count) in dates.iter().rev().take(7) {
            let bar_len = (**count).min(50);
            let bar: String = "=".repeat(bar_len);
            println!("║     {} {:4} {}{}║", date, count, bar, " ".repeat(50 - bar_len));
        }
    }

    // Top projects
    if !by_project.is_empty() {
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║  TOP PROJECTS                                                ║");
        let mut projects: Vec<_> = by_project.iter().collect();
        projects.sort_by(|a, b| b.1.cmp(a.1));
        for (project, count) in projects.iter().take(5) {
            let short_name: String = project.chars().take(40).collect();
            println!("║     {:40} {:6} msgs ║", short_name, count);
        }
    }

    println!("╚══════════════════════════════════════════════════════════════╝\n");

    Ok(())
}
