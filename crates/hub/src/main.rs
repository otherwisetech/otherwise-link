//! `otherwise-hub`: the MCP server for every installed Otherwise app.
//!
//! ```text
//! otherwise-hub            serve MCP on stdin / stdout (what an MCP client runs)
//! otherwise-hub list       the apps found, their MCP servers and tool counts (--json)
//! otherwise-hub --version
//! ```
//!
//! Add it to Claude Code with `claude mcp add otherwise-hub -- <path to otherwise-hub>`.
use otherwise_mcp_hub::{Config, Hub};

const USAGE: &str = "otherwise-hub: one MCP server for every installed Otherwise app

usage:
  otherwise-hub          serve MCP on stdin/stdout
  otherwise-hub list     show the apps found and their tools (--json for JSON)
  otherwise-hub --version

Write tools are offered only when enabled in Otherwise Apps (Settings → Hub).
Shared data: OTHERWISE_DATA_DIR, else the platform's Otherwise folder.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("serve") => {
            if let Err(e) = otherwise_mcp_hub::run_stdio() {
                eprintln!("otherwise-hub: {e}");
                std::process::exit(1);
            }
        }
        Some("list") => {
            let hub = Hub::start(Config::system());
            let json = args.iter().any(|a| a == "--json");
            let status = hub.status();
            if json {
                let v: Vec<serde_json::Value> = status
                    .iter()
                    .map(|s| serde_json::json!({"app": s.app, "name": s.name, "program": s.program, "tools": s.tools.as_ref().ok(), "error": s.tools.as_ref().err()}))
                    .collect();
                println!("{}", serde_json::json!({"apps": v, "writes": hub.writes(), "tools": hub.tool_list().len()}));
            } else {
                if status.is_empty() {
                    println!("no app MCP servers found");
                }
                for s in &status {
                    match &s.tools {
                        Ok(n) => println!("{:<10} {n} tools  ({})", s.app, s.program),
                        Err(e) => println!("{:<10} unavailable: {e}  ({})", s.app, s.program),
                    }
                }
                println!("writes: {}", if hub.writes() { "on" } else { "off" });
                println!("tools offered: {}", hub.tool_list().len());
            }
        }
        Some("-V" | "--version" | "version") => println!("otherwise-hub {}", env!("CARGO_PKG_VERSION")),
        Some("-h" | "--help" | "help") => println!("{USAGE}"),
        Some(other) => {
            eprintln!("otherwise-hub: unknown command {other:?}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}
