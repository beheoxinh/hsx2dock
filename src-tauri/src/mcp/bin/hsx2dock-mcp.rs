//! The standalone `hsx2dock-mcp` binary: the app's MCP server over stdio.
//!
//! This is what an agent's MCP client configuration points at. It builds the
//! same [`AppState`] the GUI builds — same engine, same config file, same
//! endpoints — and serves the app's whole tool catalogue over stdin/stdout.
//!
//! It deliberately does **not** start the GUI, the tray, the background
//! monitors, or the CLI bridge shim: with the GUI closed the app is not
//! managing the engine, so an agent's `engine_start` call would race a GUI that
//! is about to bring it up itself. Engine lifecycle tools still work here; they
//! just find the engine stopped and start it on demand.

use std::sync::Arc;

fn main() {
    // Logs go to stderr — stdout carries JSON-RPC and must stay clean.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let runtime = hsx2dock_lib::runtime::runtime();
    let result: Result<(), String> = runtime.block_on(async {
        let state = Arc::new(hsx2dock_lib::state::AppState::new().await);
        // Same process-wide settings the GUI seeds at launch: an engine this
        // process starts must resolve the same CLI and trust the same
        // plain-HTTP registries.
        {
            let cfg = state.config.read().await;
            hsx2dock_lib::docker::engine::set_cli_mode(&cfg.cli_mode);
            hsx2dock_lib::docker::engine::set_insecure_registries(
                hsx2dock_lib::commands::registries::insecure_registry_hosts(&cfg.registries),
            );
        }
        hsx2dock_lib::mcp::stdio::serve(state).await
    });

    if let Err(e) = result {
        eprintln!("hsx2dock-mcp: {e}");
        std::process::exit(1);
    }
}
