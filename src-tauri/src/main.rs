// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "mcp" {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for MCP server");
        if let Err(e) = rt.block_on(agent_board_lib::mcp::run_mcp_stdio()) {
            eprintln!("Agent Board MCP error: {e}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    agent_board_lib::run()
}
