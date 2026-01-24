//! State management for ai-contexters.
//!
//! Tracks processing watermarks, content hashes for deduplication,
//! and run history. Persists to `~/.ai-contexters/state.json`.
//!
//! Created by M&K (c)2026 VetCoders

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Default maximum number of stored hashes before pruning.
const DEFAULT_MAX_HASHES: usize = 50_000;

/// Record of a single extraction run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    /// When this run was executed.
    pub timestamp: DateTime<Utc>,
    /// Number of new entries added during this run.
    pub entries_added: usize,
    /// Sources processed (e.g., "claude:CodeScribe", "codex:global").
    pub sources: Vec<String>,
}

/// Persistent state for incremental processing and deduplication.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateManager {
    /// Per-source watermark: only process entries newer than this timestamp.
    pub last_processed: HashMap<String, DateTime<Utc>>,
    /// Content hashes of already-processed entries (dedup set).
    pub seen_hashes: HashSet<u64>,
    /// History of extraction runs.
    pub runs: Vec<RunRecord>,
}

impl StateManager {
    /// Returns the path to the state file: `~/.ai-contexters/state.json`
    fn state_path() -> Result<PathBuf> {
        let base = crate::store::store_base_dir()?;
        Ok(base.join("state.json"))
    }

    /// Load state from disk. Creates a fresh default state if the file
    /// does not exist or cannot be parsed.
    pub fn load() -> Self {
        let path = match Self::state_path() {
            Ok(p) => p,
            Err(_) => return Self::default(),
        };

        if !path.exists() {
            return Self::default();
        }

        let contents = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };

        serde_json::from_str(&contents).unwrap_or_default()
    }

    /// Persist current state to disk. Creates parent directories if needed.
    pub fn save(&self) -> Result<()> {
        let path = Self::state_path()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config dir: {}", parent.display()))?;
        }

        let json = serde_json::to_string_pretty(self).context("Failed to serialize state")?;

        fs::write(&path, json)
            .with_context(|| format!("Failed to write state file: {}", path.display()))?;

        Ok(())
    }

    // ========================================================================
    // Dedup API
    // ========================================================================

    /// Compute a stable content hash from entry fields.
    ///
    /// Uses `DefaultHasher` (SipHash) for fast, collision-resistant hashing.
    /// The hash is deterministic within a single binary build (which is
    /// sufficient for dedup across runs of the same version).
    pub fn content_hash(agent: &str, session_id: &str, timestamp: i64, message: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        agent.hash(&mut hasher);
        session_id.hash(&mut hasher);
        timestamp.hash(&mut hasher);
        message.hash(&mut hasher);
        hasher.finish()
    }

    /// Returns `true` if this hash has NOT been seen before (i.e., the entry is new).
    pub fn is_new(&self, hash: u64) -> bool {
        !self.seen_hashes.contains(&hash)
    }

    /// Mark a hash as seen (entry has been processed).
    pub fn mark_seen(&mut self, hash: u64) {
        self.seen_hashes.insert(hash);
    }

    // ========================================================================
    // Watermark API
    // ========================================================================

    /// Get the watermark timestamp for a given source.
    ///
    /// Returns `None` if this source has never been processed.
    pub fn get_watermark(&self, source: &str) -> Option<DateTime<Utc>> {
        self.last_processed.get(source).copied()
    }

    /// Update the watermark for a source, but only if the new timestamp
    /// is strictly newer than the existing one.
    pub fn update_watermark(&mut self, source: &str, ts: DateTime<Utc>) {
        let entry = self.last_processed.entry(source.to_string()).or_insert(ts);
        if ts > *entry {
            *entry = ts;
        }
    }

    // ========================================================================
    // Run tracking
    // ========================================================================

    /// Record a completed extraction run.
    pub fn record_run(&mut self, entries: usize, sources: Vec<String>) {
        self.runs.push(RunRecord {
            timestamp: Utc::now(),
            entries_added: entries,
            sources,
        });
    }

    // ========================================================================
    // Cleanup API
    // ========================================================================

    /// Prune the hash set to prevent unbounded growth.
    ///
    /// Keeps only the last `max_hashes` entries. Since `HashSet` has no
    /// insertion order, this drains arbitrary entries — which is acceptable
    /// because old hashes correspond to entries that would be filtered by
    /// watermarks anyway.
    ///
    /// Pass `0` to use the default maximum (`50_000`).
    pub fn prune_old_hashes(&mut self, max_hashes: usize) {
        let limit = if max_hashes == 0 {
            DEFAULT_MAX_HASHES
        } else {
            max_hashes
        };

        if self.seen_hashes.len() <= limit {
            return;
        }

        let excess = self.seen_hashes.len() - limit;
        let to_remove: Vec<u64> = self.seen_hashes.iter().take(excess).copied().collect();
        for hash in to_remove {
            self.seen_hashes.remove(&hash);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_default_state_is_empty() {
        let state = StateManager::default();
        assert!(state.last_processed.is_empty());
        assert!(state.seen_hashes.is_empty());
        assert!(state.runs.is_empty());
    }

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = StateManager::content_hash("claude", "sess-123", 1700000000, "hello world");
        let h2 = StateManager::content_hash("claude", "sess-123", 1700000000, "hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_content_hash_varies_with_input() {
        let h1 = StateManager::content_hash("claude", "sess-123", 1700000000, "hello");
        let h2 = StateManager::content_hash("claude", "sess-123", 1700000000, "world");
        assert_ne!(h1, h2);

        let h3 = StateManager::content_hash("codex", "sess-123", 1700000000, "hello");
        assert_ne!(h1, h3);

        let h4 = StateManager::content_hash("claude", "sess-456", 1700000000, "hello");
        assert_ne!(h1, h4);

        let h5 = StateManager::content_hash("claude", "sess-123", 1700000001, "hello");
        assert_ne!(h1, h5);
    }

    #[test]
    fn test_is_new_and_mark_seen() {
        let mut state = StateManager::default();
        let hash = StateManager::content_hash("claude", "s1", 100, "msg");

        assert!(state.is_new(hash));
        state.mark_seen(hash);
        assert!(!state.is_new(hash));
    }

    #[test]
    fn test_watermark_none_for_unknown_source() {
        let state = StateManager::default();
        assert_eq!(state.get_watermark("nonexistent"), None);
    }

    #[test]
    fn test_watermark_update_only_if_newer() {
        let mut state = StateManager::default();

        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 8, 0, 0).unwrap();

        state.update_watermark("claude:CodeScribe", t1);
        assert_eq!(state.get_watermark("claude:CodeScribe"), Some(t1));

        // Newer timestamp updates
        state.update_watermark("claude:CodeScribe", t2);
        assert_eq!(state.get_watermark("claude:CodeScribe"), Some(t2));

        // Older timestamp does NOT update
        state.update_watermark("claude:CodeScribe", t0);
        assert_eq!(state.get_watermark("claude:CodeScribe"), Some(t2));
    }

    #[test]
    fn test_record_run() {
        let mut state = StateManager::default();
        assert!(state.runs.is_empty());

        state.record_run(
            42,
            vec!["claude:Proj".to_string(), "codex:global".to_string()],
        );

        assert_eq!(state.runs.len(), 1);
        assert_eq!(state.runs[0].entries_added, 42);
        assert_eq!(state.runs[0].sources, vec!["claude:Proj", "codex:global"]);
    }

    #[test]
    fn test_prune_old_hashes_below_limit() {
        let mut state = StateManager::default();
        for i in 0..10u64 {
            state.mark_seen(i);
        }

        // No pruning needed — set is below limit
        state.prune_old_hashes(100);
        assert_eq!(state.seen_hashes.len(), 10);
    }

    #[test]
    fn test_prune_old_hashes_above_limit() {
        let mut state = StateManager::default();
        for i in 0..100u64 {
            state.mark_seen(i);
        }

        state.prune_old_hashes(30);
        assert_eq!(state.seen_hashes.len(), 30);
    }

    #[test]
    fn test_prune_old_hashes_default_limit() {
        let mut state = StateManager::default();
        // Just verify passing 0 uses the default and doesn't panic
        state.prune_old_hashes(0);
        assert!(state.seen_hashes.len() <= DEFAULT_MAX_HASHES);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut state = StateManager::default();
        let t = Utc.with_ymd_and_hms(2026, 1, 20, 15, 30, 0).unwrap();

        state.update_watermark("claude:TestProject", t);
        state.mark_seen(123456789);
        state.mark_seen(987654321);
        state.record_run(5, vec!["claude:TestProject".to_string()]);

        let json = serde_json::to_string_pretty(&state).unwrap();
        let restored: StateManager = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.get_watermark("claude:TestProject"), Some(t));
        assert!(!restored.is_new(123456789));
        assert!(!restored.is_new(987654321));
        assert!(restored.is_new(111111111));
        assert_eq!(restored.runs.len(), 1);
        assert_eq!(restored.runs[0].entries_added, 5);
    }

    #[test]
    fn test_state_path_is_under_store() {
        // Verify the path structure (won't fail on CI with no home dir
        // because state_path returns Result)
        if let Ok(path) = StateManager::state_path() {
            assert!(path.to_string_lossy().contains(".ai-contexters"));
            assert!(path.to_string_lossy().ends_with("state.json"));
        }
    }
}
