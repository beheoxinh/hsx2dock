use std::process::ExitCode;
use std::rc::Rc;

use adw::prelude::*;

use docker_manager::config::AppConfig;
use docker_manager::runtime::runtime;
use docker_manager::ui::app_state::AppState;
use docker_manager::ui::window::MainWindow;

const APP_ID: &str = "com.github.docker_manager";

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("docker_manager=info,warn")),
        )
        .init();

    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("docker-manager {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!(
            "docker-manager {} — GUI for managing Docker resources\n\n\
             USAGE:\n    docker-manager [OPTIONS]\n\n\
             OPTIONS:\n    -h, --help       Show this help\n    -V, --version    Show version\n\n\
             The app detects a running Docker socket (~/.docker/desktop/docker.sock or\n\
             /var/run/docker.sock) and can start/stop the daemon via systemd user units.",
            env!("CARGO_PKG_VERSION")
        );
        return ExitCode::SUCCESS;
    }

    // Config is small and read once at startup; block on the shared runtime.
    let config = runtime().block_on(AppConfig::load()).unwrap_or_default();
    let state = AppState::new(config);

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(include_str!("../data/style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });

    let state_for_activate = state.clone();
    app.connect_activate(move |app| {
        if let Some(existing) = app.active_window() {
            existing.present();
            return;
        }
        let window = MainWindow::new(app, state_for_activate.clone());
        window.window().present();
    });

    // The runtime must outlive the GTK loop; keep an explicit handle alive.
    let _runtime_guard = Rc::new(runtime());

    app.run().into()
}
