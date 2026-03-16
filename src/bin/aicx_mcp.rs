//! Standalone MCP server binary for aicx.
//!
//! Exposes aicx search, rank, refs, and store as MCP tools.
//! Supports stdio (default) and streamable HTTP transports.
//!
//! Usage:
//!   aicx-mcp                          # stdio transport
//!   aicx-mcp --transport sse          # HTTP on port 8044
//!   aicx-mcp --transport sse --port 9000
//!
//! Vibecrafted with AI Agents by VetCoders (c)2026 VetCoders

use clap::Parser;

/// aicx MCP server — AI session context as MCP tools
#[derive(Parser)]
#[command(name = "aicx-mcp")]
#[command(author = "M&K (c)2026 VetCoders")]
#[command(version)]
struct Args {
    /// Transport: stdio (default) or sse
    #[arg(long, default_value = "stdio", value_parser = ["stdio", "sse"])]
    transport: String,

    /// Port for SSE/HTTP transport
    #[arg(long, default_value = "8044")]
    port: u16,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match args.transport.as_str() {
            "sse" => ai_contexters::mcp::run_sse(args.port).await,
            _ => ai_contexters::mcp::run_stdio().await,
        }
    })
}
