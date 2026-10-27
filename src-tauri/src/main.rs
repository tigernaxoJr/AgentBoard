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

    if args.len() > 1 && args[1] == "runner" {
        let mut config = agent_board_lib::runner::RunnerConfig {
            agent: "agy".to_string(),
            server: agent_board_lib::runner::DEFAULT_SERVER_URL.to_string(),
            interval_secs: agent_board_lib::runner::DEFAULT_INTERVAL_SECS,
            token: None,
            token_file: None,
            dry_run: false,
            dry_run_seconds: 3,
            once: false,
        };

        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--agent" if i + 1 < args.len() => {
                    config.agent = args[i + 1].clone();
                    i += 2;
                }
                "--server" if i + 1 < args.len() => {
                    config.server = args[i + 1].clone();
                    i += 2;
                }
                "--interval" if i + 1 < args.len() => {
                    if let Ok(v) = args[i + 1].parse() {
                        config.interval_secs = v;
                    }
                    i += 2;
                }
                "--token" if i + 1 < args.len() => {
                    config.token = Some(args[i + 1].clone());
                    i += 2;
                }
                "--token-file" if i + 1 < args.len() => {
                    config.token_file = Some(args[i + 1].clone());
                    i += 2;
                }
                "--dry-run" => {
                    config.dry_run = true;
                    i += 1;
                }
                "--dry-run-seconds" if i + 1 < args.len() => {
                    if let Ok(v) = args[i + 1].parse() {
                        config.dry_run_seconds = v;
                    }
                    i += 2;
                }
                "--once" => {
                    config.once = true;
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for Runner");
        if let Err(e) = rt.block_on(agent_board_lib::runner::run_runner(config)) {
            eprintln!("Agent Board Runner error: {e}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    agent_board_lib::run()
}
