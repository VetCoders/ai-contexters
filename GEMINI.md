# AI Contexters (aicx)

## Project Overview

**ai-contexters** is a Rust-based CLI tool (`aicx`) designed for memory extraction and context distillation for AI agent sessions. It acts as a bridge between active AI agent workflows and persistent memory by turning local agent logs into structured, readable contexts.

**Core Capabilities:**
- Extracts clean, deduplicated timelines from agent sessions.
- Chunks and stores "agent-readable" context in `~/.ai-contexters/`.
- Bootstraps repository-level context (`.ai-context/`) to bring new agents up to speed quickly.
- Synchronizes extracted contexts into a vector memory store (memex).
- Redacts secrets by default to ensure safe context sharing.

**Supported Agent Sources:**
- Claude Code (`~/.claude/projects/*/*.jsonl`)
- Codex (`~/.codex/history.jsonl`)
- Gemini CLI (`~/.gemini/tmp/<hash>/chats/session-*.json`)

## Architecture & Technologies

- **Language:** Rust (Edition 2024)
- **Key Crates:** `clap` (CLI), `tokio` (async runtime), `serde` / `serde_json` (serialization), `axum` (dashboard server), `regex` (redaction).
- **Structure:**
  - `src/main.rs`: CLI entry point (`aicx`).
  - `src/lib.rs` & modules (`chunker.rs`, `dashboard.rs`, `memex.rs`, `redact.rs`, `sanitize.rs`, `sources.rs`, `store.rs`): Core business logic.
  - `docs/`: In-depth documentation covering architecture, commands, distillation, redaction, and store layout.

## Building and Running

The project uses standard Cargo commands.

**Install locally:**
```bash
cargo install --path .
```

**Build:**
```bash
cargo build
```

**Test:**
```bash
cargo test
```

## Common Usage Examples

**Extract all sessions from the last 4 hours:**
```bash
aicx all -H 4
```

**Bootstrap a repository context and run an agent:**
```bash
aicx init --agent codex --no-confirm --action "Map the repo and propose next steps"
```
*(Note: `init` expects `loct` (loctree) to be available in your `PATH`)*

**Extract Claude Code sessions incrementally:**
```bash
aicx claude -p ProjectName -H 24 --incremental
```

## Development Conventions & VetCoders Charter

This project is a VetCoders production ("Vibecrafted with AI Agents by VetCoders"). When working on this repository, strictly adhere to the **VetCoders Global Agent Charter**:

1. **Be an explorer, not a caretaker:** Prefer discovery and bold simplification over patching scar tissue. Use tools like `loctree-mcp` (`repo-view`, `slice`, `impact`) for structural mapping before making changes.
2. **Backward compatibility is optional:** Clean architecture and runtime truth beat preserving old abstractions.
3. **Vibecrafting is valid:** Code is craft. Shape systems intelligently.
4. **DoU is law:** "Done" means repo health, runtime health, install path, and customer readiness.
5. **Living Tree Rule:** Expect concurrent edits. Re-read files if time has passed. Do not use git worktrees for active implementation.
6. **Tooling Priorities:**
   - **Quality Gates:** Use `cargo clippy -- -D warnings` and `cargo test` to verify changes.
   - **Structural Mapping:** Always use `loctree` tools before `grep` for first-pass understanding.
7. **Documentation:** Keep the `docs/` directory up-to-date with architectural and operational realities. If making structural changes, ensure `ARCHITECTURE.md` and `COMMANDS.md` reflect them.