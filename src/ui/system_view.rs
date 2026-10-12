use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::docker::cli_bridge::{self as engine_cli, CliLink, CliStatus, CONTEXT_NAME};
use crate::docker::engine::{self, EnginePaths, DEFAULT_RELEASE};
use crate::docker::models::{DaemonState, DfReport, EngineInfo};
use crate::ui::common::{copy_to_clipboard, format_bytes};

/// Shown before the engine is downloaded: what it is and how this app uses it.
const INSTALL_DESC: &str = "This app ships and runs its own Docker Engine — the official \
static rootless binaries, downloaded once into the app's private folder and started as a \
child of this app on a private unix socket. The system's Docker is neither required nor \
touched. Downloads are checksum-verified before extraction, and your containers and images \
live in a separate data folder that survives engine updates.";

const STARTING_DESC: &str = "rootlesskit is bringing up dockerd on the app's private unix socket.";

const NET_BANNER: &str = "Rootless networking helper (pasta or slirp4netns) not found — \
the engine may fail to start.";

/// Engine page: one libadwaita surface per real engine state.
pub struct SystemView {
    widget: gtk::Box,
    banner: adw::Banner,
    stack: gtk::Stack,
    status_page: adw::StatusPage,
    status_child: gtk::Stack,
    progress_label: gtk::Label,
    progress_bar: gtk::ProgressBar,
    progress_spinner: gtk::Spinner,
    version_row: adw::ActionRow,
    api_row: adw::ActionRow,
    os_row: adw::ActionRow,
    socket_row: adw::ActionRow,
    data_row: adw::ActionRow,
    log_row: adw::ActionRow,
    location_row: adw::ActionRow,
    resources_row: adw::ActionRow,
    disk_row: adw::ActionRow,
    /// Dashboard metric tiles (value labels), updated as data arrives.
    metric_containers: gtk::Label,
    metric_images: gtk::Label,
    metric_volumes: gtk::Label,
    metric_cpu: gtk::Label,
    metric_memory: gtk::Label,
    metric_disk: gtk::Label,
    start_stop_btn: gtk::Button,
    /// Inline spinner shown inside the Start/Stop button while it runs.
    action_spinner: gtk::Spinner,
    /// "Docker CLI" linkage panel.
    cli_row: adw::ActionRow,
    cli_pill: gtk::Label,
    cli_toggle: gtk::Switch,
    state: Rc<AppState>,
}

impl SystemView {
    pub fn new(state: Rc<AppState>) -> Rc<Self> {
        // ---- preflight banner -------------------------------------------------
        let banner = adw::Banner::new(NET_BANNER);
        banner.set_revealed(false);

        // ---- status surface (not installed / installing / starting / error) ---
        let install_btn = gtk::Button::with_label("Download Engine");
        install_btn.add_css_class("suggested-action");
        install_btn.set_tooltip_text(Some("Download the bundled Docker engine"));
        let learn_btn = gtk::Button::with_label("Learn More");
        learn_btn.add_css_class("flat");
        learn_btn.set_tooltip_text(Some("What is Docker Engine?"));
        let install_child = button_box(&[&install_btn, &learn_btn]);

        let progress_label = gtk::Label::builder()
            .label("")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build();
        let progress_bar = gtk::ProgressBar::builder().width_request(320).build();
        let progress_spinner = gtk::Spinner::new();
        progress_spinner.set_visible(false);
        let progress_child = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .halign(gtk::Align::Center)
            .build();
        progress_child.append(&progress_spinner);
        progress_child.append(&progress_bar);
        progress_child.append(&progress_label);

        let retry_btn = gtk::Button::with_label("Try Again");
        retry_btn.add_css_class("suggested-action");
        let err_log_btn = gtk::Button::with_label("View Log");
        let err_remove_btn = gtk::Button::with_label("Remove Engine…");
        err_remove_btn.add_css_class("destructive-action");
        let error_child = button_box(&[&retry_btn, &err_log_btn, &err_remove_btn]);

        let status_child = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        status_child.add_named(&install_child, Some("install"));
        status_child.add_named(&progress_child, Some("progress"));
        status_child.add_named(&error_child, Some("error"));

        let status_page = adw::StatusPage::builder()
            .icon_name("drive-harddisk-symbolic")
            .title("Docker Engine")
            .description(INSTALL_DESC)
            .build();
        status_page.set_child(Some(&status_child));

        // ---- preferences surface (installed: stopped / running) ---------------
        let version_row = copy_row("Version");
        let api_row = plain_row("API version");
        let os_row = plain_row("Host");
        let socket_row = copy_row("Socket");
        let data_row = copy_row("Data folder");
        let log_row = copy_row("Log file");
        let location_row = copy_row("Installed at");
        let resources_row = plain_row("Containers");
        let disk_row = plain_row("Disk usage");

        let view_log_btn = gtk::Button::with_label("View");
        view_log_btn.add_css_class("flat");
        view_log_btn.set_valign(gtk::Align::Center);
        view_log_btn.set_tooltip_text(Some("Open the engine log"));
        log_row.add_suffix(&view_log_btn);

        let action_spinner = gtk::Spinner::new();
        action_spinner.set_visible(false);
        let start_stop_btn = gtk::Button::builder()
            .child(&button_with_spinner("Start Engine", &action_spinner))
            .build();
        start_stop_btn.add_css_class("suggested-action");
        start_stop_btn.set_valign(gtk::Align::Center);
        let refresh_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh engine information")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let prune_btn = gtk::Button::with_label("System Prune");
        prune_btn.add_css_class("destructive-action");
        prune_btn.set_valign(gtk::Align::Center);
        prune_btn.set_tooltip_text(Some("Remove unused data (confirm first)"));
        let remove_btn = gtk::Button::with_label("Remove Engine…");
        remove_btn.add_css_class("destructive-action");
        remove_btn.set_valign(gtk::Align::Center);

        let engine_group = adw::PreferencesGroup::builder()
            .title("Engine")
            .description("The rootless Docker Engine this app downloaded and runs itself")
            .build();
        engine_group.add(&version_row);
        engine_group.add(&api_row);
        engine_group.add(&os_row);
        engine_group.add(&socket_row);
        engine_group.add(&data_row);
        engine_group.add(&log_row);
        engine_group.add(&location_row);

        let resources_group = adw::PreferencesGroup::builder().title("Resources").build();
        resources_group.add(&resources_row);
        resources_group.add(&disk_row);

        let actions_group = adw::PreferencesGroup::builder().title("Actions").build();
        actions_group.add(&action_row(
            "Engine",
            "Start or stop the bundled engine",
            &start_stop_btn,
        ));
        actions_group.add(&action_row(
            "Refresh",
            "Re-read engine information",
            &refresh_btn,
        ));
        actions_group.add(&action_row(
            "Maintenance",
            "Remove unused images, containers and networks",
            &prune_btn,
        ));
        actions_group.add(&action_row(
            "Remove engine",
            "Delete the downloaded binaries — container data is kept",
            &remove_btn,
        ));

        // ---- dashboard -----------------------------------------------------
        // A responsive grid of metric tiles, then the detail groups side by
        // side, so the page uses the full window instead of one narrow column.
        let (containers_tile, metric_containers) =
            crate::ui::common::metric_tile("—", "Containers running");
        let (images_tile, metric_images) = crate::ui::common::metric_tile("—", "Images");
        let (volumes_tile, metric_volumes) = crate::ui::common::metric_tile("—", "Volumes");
        let (cpu_tile, metric_cpu) = crate::ui::common::metric_tile("—", "CPU cores");
        let (memory_tile, metric_memory) = crate::ui::common::metric_tile("—", "Memory");
        let (disk_tile, metric_disk) = crate::ui::common::metric_tile("—", "Disk used");

        let metric_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        metric_row.set_homogeneous(false);
        for tile in [
            &containers_tile,
            &images_tile,
            &volumes_tile,
            &cpu_tile,
            &memory_tile,
            &disk_tile,
        ] {
            tile.set_hexpand(true);
            metric_row.append(tile);
        }

        let columns = crate::ui::common::two_columns(&resources_group, &actions_group);

        let dashboard = gtk::Box::new(gtk::Orientation::Vertical, 18);
        dashboard.set_margin_top(12);
        dashboard.set_margin_bottom(24);
        dashboard.set_margin_start(12);
        dashboard.set_margin_end(12);
        dashboard.append(&metric_row);
        dashboard.append(&columns);
        let dashboard_clamp = crate::ui::common::clamp_grid(&dashboard);
        let dashboard_group = adw::PreferencesGroup::new();
        dashboard_group.add(&dashboard_clamp);

        let prefs_page = adw::PreferencesPage::new();
        prefs_page.set_vexpand(true);
        prefs_page.add(&dashboard_group);
        prefs_page.add(&engine_group);

        // ---- Docker CLI linkage panel ----------------------------------------
        // Makes the system `docker` CLI talk to our private engine. Off by
        // default; re-checked every 10s while enabled (see start_cli_watch).
        let cli_pill = gtk::Label::new(None);
        crate::ui::common::set_pill(
            &cli_pill,
            "Disconnected",
            crate::ui::common::PillTone::Stopped,
        );
        cli_pill.set_valign(gtk::Align::Center);
        let cli_toggle = gtk::Switch::new();
        cli_toggle.set_valign(gtk::Align::Center);
        let cli_row = adw::ActionRow::builder()
            .title("Docker CLI")
            .subtitle("Connect the system `docker` command to this engine")
            .build();
        cli_row.add_prefix(&cli_pill);
        cli_row.add_suffix(&cli_toggle);
        cli_row.set_activatable_widget(Some(&cli_toggle));
        let cli_group = adw::PreferencesGroup::builder()
            .title("Docker CLI")
            .description("Let the system `docker` command talk to this app's engine")
            .build();
        cli_group.add(&cli_row);
        prefs_page.add(&cli_group);

        // ---- shell ------------------------------------------------------------
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vexpand(true)
            .build();
        stack.add_named(&status_page, Some("status"));
        stack.add_named(&prefs_page, Some("prefs"));

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        widget.append(&banner);
        widget.append(&crate::ui::common::clamp_page(&stack));

        let view = Rc::new(Self {
            widget,
            banner,
            stack,
            status_page,
            status_child,
            progress_label,
            progress_bar,
            progress_spinner,
            version_row,
            api_row,
            os_row,
            socket_row,
            data_row,
            log_row,
            location_row,
            resources_row,
            disk_row,
            metric_containers,
            metric_images,
            metric_volumes,
            metric_cpu,
            metric_memory,
            metric_disk,
            start_stop_btn,
            action_spinner,
            cli_row,
            cli_pill,
            cli_toggle,
            state,
        });

        view.connect_signals(
            &install_btn,
            &learn_btn,
            &retry_btn,
            &err_log_btn,
            &err_remove_btn,
            &view_log_btn,
            &refresh_btn,
            &prune_btn,
            &remove_btn,
            &view.start_stop_btn,
        );
        view.refresh_daemon_state();
        view.connect_cli_toggle();
        view.start_cli_watch();
        view.refresh_cli_state();
        view
    }

    /// Wire the Docker CLI toggle: persist the flag, then reconcile at once.
    fn connect_cli_toggle(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.cli_toggle.connect_active_notify(move |sw| {
            let Some(view) = weak.upgrade() else {
                return;
            };
            let enabled = sw.is_active();
            view.state.update_config(|c| c.cli_bridge_enabled = enabled);
            view.refresh_cli_state();
        });
    }

    /// Poll the Docker CLI linkage every 10s so an external change (user ran
    /// `docker context use other`) or a missing symlink is repaired/shown
    /// without the user touching anything.
    fn start_cli_watch(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(10, move || {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            view.refresh_cli_state();
            glib::ControlFlow::Continue
        });
    }

    /// Reconcile the CLI link (when enabled) and repaint the panel.
    ///
    /// The `docker`-spawning work (context create/use, symlink) runs on the
    /// persistent engine thread, never the GTK main loop; the widget update is
    /// bounced back through `main_loop_channel`.
    pub fn refresh_cli_state(self: &Rc<Self>) {
        let cfg = self.state.config.borrow().clone();
        let socket = self.state.engine.socket().to_path_buf();
        // Liveness drives teardown: a symlink/context pointing at a dead
        // socket is misleading, so reconcile removes it when the engine is
        // down (and re-creates it once the engine comes back).
        let engine_live = matches!(self.state.daemon_state(), DaemonState::Running);
        let weak = Rc::downgrade(self);
        let tx = crate::ui::common::main_loop_channel(move |st: CliStatus| {
            if let Some(view) = weak.upgrade() {
                view.render_cli_state(&st);
            }
        });
        crate::runtime::spawn(async move {
            let st = crate::runtime::engine_op(move || {
                let _ = engine_cli::reconcile(
                    &socket,
                    cfg.cli_bridge_enabled,
                    cfg.cli_bridge_set_default,
                    engine_live,
                );
                engine_cli::status(&socket)
            })
            .await;
            if let Ok(st) = st {
                let _ = tx.send(st).await;
            }
        });
    }

    /// Paint the "Docker CLI" row from a fresh status snapshot.
    fn render_cli_state(&self, st: &CliStatus) {
        use crate::ui::common::{set_pill, PillTone};
        let bridge_enabled = self.state.config.borrow().cli_bridge_enabled;
        let engine_live = matches!(self.state.daemon_state(), DaemonState::Running);
        let (tone, text) = if !bridge_enabled {
            (PillTone::Stopped, "Disabled")
        } else if !engine_live {
            (PillTone::Stopped, "Engine stopped")
        } else {
            match st.link {
                CliLink::Connected => (PillTone::Running, "Connected"),
                CliLink::Partial => (PillTone::Warn, "Partial"),
                CliLink::Disconnected => (PillTone::Stopped, "Disconnected"),
            }
        };
        set_pill(&self.cli_pill, text, tone);
        let sub = if !bridge_enabled {
            "Not linked. Turn the switch on to connect the system `docker` command.".to_string()
        } else if !engine_live {
            "Engine is stopped — the mapping is removed; the CLI cannot reach it until it starts."
                .to_string()
        } else {
            match st.link {
                CliLink::Connected => "`docker` in any terminal reaches this engine.".to_string(),
                CliLink::Partial if st.symlink_ok && !st.context_is_default => {
                    match &st.active_context {
                        Some(c) => format!(
                            "Context active is \"{c}\". Use `docker --context {} ...` or enable \"set as default\".",
                            CONTEXT_NAME
                        ),
                        None => format!(
                            "Use `docker --context {} ...`, or enable \"set as default\".",
                            CONTEXT_NAME
                        ),
                    }
                }
                CliLink::Partial => format!(
                    "Symlink {} in place; context not selected.",
                    st.xdg_path.display()
                ),
                CliLink::Disconnected => {
                    "Not connected — the bridge repairs symlink and context on the next tick."
                        .to_string()
                }
            }
        };
        self.cli_row.set_subtitle(&sub);
        self.cli_toggle.set_active(bridge_enabled);
        self.cli_toggle.set_sensitive(engine_live || bridge_enabled);
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.widget
    }

    #[allow(clippy::too_many_arguments)]
    fn connect_signals(
        self: &Rc<Self>,
        install_btn: &gtk::Button,
        learn_btn: &gtk::Button,
        retry_btn: &gtk::Button,
        err_log_btn: &gtk::Button,
        err_remove_btn: &gtk::Button,
        view_log_btn: &gtk::Button,
        refresh_btn: &gtk::Button,
        prune_btn: &gtk::Button,
        remove_btn: &gtk::Button,
        start_stop_btn: &gtk::Button,
    ) {
        let state = self.state.clone();
        install_btn.connect_clicked(move |_| download_engine(state.clone()));

        // The page's primary action mirrors the header button: install when the
        // engine is missing, otherwise start/stop it, with inline feedback.
        let weak = Rc::downgrade(self);
        start_stop_btn.connect_clicked(move |btn| {
            let Some(view) = weak.upgrade() else {
                return;
            };
            let state = view.state.clone();
            match state.daemon_state() {
                DaemonState::NotInstalled => {
                    download_engine(state.clone());
                    return;
                }
                DaemonState::Stopped | DaemonState::Error(_) => {
                    set_button_text(btn, "Starting…");
                    state.notify(AppEvent::DaemonState(DaemonState::Starting));
                }
                _ => {
                    set_button_text(btn, "Stopping…");
                }
            }
            state.toggle_daemon();
        });

        learn_btn.connect_clicked(|_| open_docs());

        let state = self.state.clone();
        retry_btn.connect_clicked(move |_| state.toggle_daemon());

        let state = self.state.clone();
        err_log_btn.connect_clicked(move |_| open_log(state.engine.paths()));

        let state = self.state.clone();
        let parent = err_remove_btn.clone().upcast::<gtk::Widget>();
        err_remove_btn
            .connect_clicked(move |_| confirm_remove_engine(state.clone(), parent.clone()));

        let state = self.state.clone();
        view_log_btn.connect_clicked(move |_| open_log(state.engine.paths()));

        let state = self.state.clone();
        refresh_btn.connect_clicked(move |_| state.refresh_daemon());

        let state = self.state.clone();
        let parent = prune_btn.clone().upcast::<gtk::Widget>();
        prune_btn.connect_clicked(move |_| confirm_system_prune(state.clone(), parent.clone()));

        let state = self.state.clone();
        let parent = remove_btn.clone().upcast::<gtk::Widget>();
        remove_btn.connect_clicked(move |_| confirm_remove_engine(state.clone(), parent.clone()));

        let state = self.state.clone();
        let busy_label = if matches!(state.daemon_state(), DaemonState::Running) {
            "Stopping engine…"
        } else {
            "Starting engine…"
        };
        self.set_power_busy(busy_label);
        self.start_stop_btn.connect_clicked(move |_btn| {
            state.toggle_daemon();
        });
    }

    /// Map the current daemon state onto the right surface.
    pub fn refresh_daemon_state(&self) {
        // A present engine means any in-flight install finished.
        if self.state.engine.is_installed() {
            self.state.set_installing(false);
        }
        self.check_net_helper();
        self.render_engine_info();

        let state = self.state.daemon_state();
        match &state {
            DaemonState::NotInstalled => {
                self.stack.set_visible_child_name("status");
                self.status_child.set_visible_child_name("install");
                self.status_page
                    .set_icon_name(Some("drive-harddisk-symbolic"));
                self.status_page.set_title("Docker Engine not installed");
                self.status_page.set_description(Some(INSTALL_DESC));
            }
            DaemonState::Installing => {
                self.stack.set_visible_child_name("status");
                self.status_child.set_visible_child_name("progress");
                self.status_page
                    .set_icon_name(Some("drive-harddisk-symbolic"));
                self.status_page.set_title("Downloading Docker Engine");
                self.status_page.set_description(Some(
                    "Verifying checksums, then extracting into the app's own folder.",
                ));
                self.progress_spinner.set_visible(false);
                self.progress_bar.set_visible(true);
            }
            DaemonState::Starting => {
                self.stack.set_visible_child_name("status");
                self.status_child.set_visible_child_name("progress");
                self.status_page
                    .set_icon_name(Some("content-loading-symbolic"));
                self.status_page.set_title("Starting engine…");
                self.status_page.set_description(Some(STARTING_DESC));
                self.progress_bar.set_visible(false);
                self.progress_label.set_label("");
                self.progress_spinner.set_visible(true);
                self.progress_spinner.start();
            }
            DaemonState::Stopping => {
                self.stack.set_visible_child_name("status");
                self.status_child.set_visible_child_name("progress");
                self.status_page
                    .set_icon_name(Some("content-loading-symbolic"));
                self.status_page.set_title("Stopping engine…");
                self.status_page
                    .set_description(Some("Waiting for dockerd to release the socket."));
                self.progress_bar.set_visible(false);
                self.progress_label.set_label("");
                self.progress_spinner.set_visible(true);
                self.progress_spinner.start();
            }
            DaemonState::Running => {
                self.show_prefs("Stop Engine", true);
            }
            DaemonState::Stopped => {
                self.show_prefs("Start Engine", false);
            }
            DaemonState::Error(msg) => {
                let msg = msg.clone();
                self.stack.set_visible_child_name("status");
                self.status_child.set_visible_child_name("error");
                self.status_page
                    .set_icon_name(Some("dialog-warning-symbolic"));
                self.status_page.set_title("Engine failed to start");
                self.status_page.set_description(Some(&msg));
                self.progress_spinner.stop();
            }
        }
    }

    /// Switch to the preferences surface with the right power action.
    fn show_prefs(&self, power_label: &str, running: bool) {
        self.stack.set_visible_child_name("prefs");
        self.progress_spinner.stop();
        self.action_spinner.stop();
        self.action_spinner.set_visible(false);
        set_button_text(&self.start_stop_btn, power_label);
        self.start_stop_btn.set_sensitive(true);
        if running {
            self.start_stop_btn.remove_css_class("suggested-action");
            self.start_stop_btn.add_css_class("destructive-action");
        } else {
            self.start_stop_btn.remove_css_class("destructive-action");
            self.start_stop_btn.add_css_class("suggested-action");
        }
    }

    /// Show an in-button spinner and busy label while a start/stop runs.
    fn set_power_busy(&self, label: &str) {
        set_button_text(&self.start_stop_btn, label);
        self.start_stop_btn.set_sensitive(false);
        self.action_spinner.set_visible(true);
        self.action_spinner.start();
    }

    /// Fill the engine-fact rows from the on-disk engine and the pinned release.
    fn render_engine_info(&self) {
        let paths = self.state.engine.paths();
        let bundled = DEFAULT_RELEASE.version;
        let version = match self.state.engine.installed_version() {
            Some(v) if v == bundled => format!("{v} (bundled, up to date)"),
            Some(v) => format!("{v} — update available: {bundled}"),
            None => format!("{bundled} (not downloaded)"),
        };
        self.version_row.set_subtitle(&version);
        self.socket_row
            .set_subtitle(&paths.socket.display().to_string());
        self.data_row
            .set_subtitle(&paths.data_root.display().to_string());
        self.log_row
            .set_subtitle(&paths.log_file.display().to_string());
        self.location_row
            .set_subtitle(&paths.current_dir.display().to_string());
    }

    /// Reveal the preflight banner when rootless networking would fail.
    fn check_net_helper(&self) {
        let paths = self.state.engine.paths();
        let missing = self.state.engine.is_installed() && engine::missing_net_helper(paths);
        self.banner.set_revealed(missing);
    }

    /// Update the download progress bar/label (called on the GTK main loop).
    pub fn set_engine_progress(&self, label: &str, fraction: Option<f64>) {
        // Show the progress page immediately — the 2s state poll would
        // otherwise leave the user staring at a dead "Download Engine" button.
        self.stack.set_visible_child_name("status");
        self.status_child.set_visible_child_name("progress");
        self.status_page
            .set_icon_name(Some("drive-harddisk-symbolic"));
        self.status_page.set_title("Downloading Docker Engine");
        self.progress_spinner.set_visible(false);
        self.progress_spinner.set_visible(true);
        self.progress_spinner.start();
        match fraction {
            Some(f) if f > 0.0 && f <= 1.0 => {
                self.progress_bar.set_visible(true);
                self.progress_bar.set_fraction(f);
                self.progress_label
                    .set_label(&format!("{label} — {}%", (f * 100.0).round() as u32));
            }
            // Unknown or zero total (first tick, or a step with no content-length):
            // pulse so the bar still shows activity instead of vanishing.
            _ => {
                self.progress_bar.set_visible(true);
                self.progress_bar.pulse();
                self.progress_label.set_label(label);
            }
        }
    }

    /// Render engine details reported by the daemon.
    pub fn set_engine_info(&self, info: &EngineInfo, socket: Option<&str>) {
        self.api_row.set_subtitle(&info.api_version);
        self.os_row.set_subtitle(&format!(
            "{} ({}) · {} cores · {} · kernel {}",
            info.os,
            info.arch,
            info.n_cpu,
            format_bytes(info.mem_total.max(0) as u64),
            info.kernel,
        ));
        self.resources_row.set_subtitle(&format!(
            "{} running · {} paused · {} stopped · {} images",
            info.containers_running, info.containers_paused, info.containers_stopped, info.images,
        ));
        self.metric_containers.set_label(&format!(
            "{}",
            info.containers_running + info.containers_paused + info.containers_stopped
        ));
        self.metric_images.set_label(&format!("{}", info.images));
        self.metric_cpu.set_label(&format!("{}", info.n_cpu));
        self.metric_memory
            .set_label(&format_bytes(info.mem_total.max(0) as u64));
        if let Some(socket) = socket {
            self.socket_row.set_subtitle(socket);
        }
    }

    /// Render disk usage reported by `docker system df`.
    pub fn set_df(&self, report: &DfReport) {
        self.disk_row.set_subtitle(&format!(
            "Images {} ({}) · Containers {} ({}) · Volumes {} ({})",
            report.images_count,
            format_bytes(report.images_size),
            report.containers_count,
            format_bytes(report.containers_size),
            report.volumes_count,
            format_bytes(report.volumes_size),
        ));
        self.metric_volumes
            .set_label(&format!("{}", report.volumes_count));
        let total = report.images_size + report.containers_size + report.volumes_size;
        self.metric_disk.set_label(&format_bytes(total));
    }
}

// ---------------------------------------------------------------------------
// Free-standing UI helpers and actions (no `Rc<Self>` needed).
// ---------------------------------------------------------------------------

/// A button child: an optional spinner (hidden by default) plus a text label.
fn button_with_spinner(label: &str, spinner: &gtk::Spinner) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    row.append(spinner);
    row.append(&gtk::Label::new(Some(label)));
    row
}

/// Replace the text of a button built by [`button_with_spinner`].
fn set_button_text(button: &gtk::Button, text: &str) {
    if let Some(row) = button.child().and_downcast::<gtk::Box>() {
        if let Some(label) = row.last_child().and_downcast::<gtk::Label>() {
            label.set_label(text);
        }
    }
}

/// A centred horizontal row of buttons.
fn button_box(buttons: &[&gtk::Button]) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    for b in buttons {
        row.append(*b);
    }
    row
}

/// An info row whose value is selectable and copyable via a trailing button.
fn copy_row(title: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle("—")
        .subtitle_selectable(true)
        .build();
    let btn = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let src = row.clone();
    btn.connect_clicked(move |b| {
        let text = src.subtitle().map(|s| s.to_string()).unwrap_or_default();
        copy_to_clipboard(b.upcast_ref::<gtk::Widget>(), &text);
    });
    row.add_suffix(&btn);
    row
}

/// An info row with no value yet.
fn plain_row(title: &str) -> adw::ActionRow {
    adw::ActionRow::builder().title(title).subtitle("—").build()
}

/// A titled row whose action is a trailing button.
fn action_row(title: &str, subtitle: &str, button: &gtk::Button) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    row.add_suffix(button);
    row
}

/// Download, verify and install the bundled engine on a worker thread.
fn download_engine(state: Rc<AppState>) {
    state.set_installing(true);
    state.notify(AppEvent::Busy(RefreshScope::Daemon));
    state.notify(AppEvent::EngineProgress(
        "Starting download…".into(),
        Some(0.0),
    ));
    let tx = state.sender();
    let engine = state.engine.clone();
    crate::runtime::spawn(async move {
        let progress_tx = tx.clone();
        let mut last_emit = std::time::Instant::now();
        let result = engine::install_release(engine.paths(), &DEFAULT_RELEASE, move |p| {
            // Throttle UI updates to ~10/s so the main loop is not flooded.
            if last_emit.elapsed() >= std::time::Duration::from_millis(100)
                || p.fraction == Some(1.0)
            {
                last_emit = std::time::Instant::now();
                let _ = progress_tx
                    .send_blocking(AppEvent::EngineProgress(p.label.clone(), p.fraction));
            }
        })
        .await;
        match result {
            Ok(()) => {
                let _ = tx.send(AppEvent::Toast("Engine downloaded".into())).await;
            }
            Err(e) => {
                let _ = tx
                    .send(AppEvent::Error(format!("Engine download failed: {e}")))
                    .await;
            }
        }
        let _ = tx.send(AppEvent::EngineChanged).await;
    });
}

/// Open the engine log file in the default handler.
fn open_log(paths: &EnginePaths) {
    let _ = std::process::Command::new("xdg-open")
        .arg(&paths.log_file)
        .spawn();
}

/// Open the official Engine docs in the default browser.
fn open_docs() {
    let _ = std::process::Command::new("xdg-open")
        .arg(engine::DOCKER_INSTALL_DOCS_URL)
        .spawn();
}

/// Confirm then delete the downloaded engine versions (optionally with data).
fn confirm_remove_engine(state: Rc<AppState>, parent: gtk::Widget) {
    let dlg = adw::AlertDialog::builder()
        .title("Remove bundled engine?")
        .body("This deletes the engine binaries the app downloaded. Container and image data is kept unless you also choose \"Also delete data\".")
        .build();
    dlg.add_response("cancel", "Cancel");
    dlg.add_response("keep", "Remove Engine");
    dlg.add_response("clear", "Also delete data");
    dlg.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
    dlg.set_response_appearance("keep", adw::ResponseAppearance::Destructive);

    dlg.connect_response(None, move |_, response| {
        if response == "cancel" {
            return;
        }
        let clear_data = response == "clear";
        let state = state.clone();
        let tx = state.sender();
        state.notify(AppEvent::Busy(RefreshScope::Daemon));
        state.notify(AppEvent::Toast("Removing engine…".into()));
        state.set_installing(false);
        let engine = state.engine.clone();
        crate::runtime::spawn(async move {
            let paths = engine.paths().clone();
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let _ = engine.stop();
                if paths.versions_dir.exists() {
                    std::fs::remove_dir_all(&paths.versions_dir)?;
                }
                let _ = std::fs::remove_dir_all(&paths.current_dir);
                if clear_data && paths.data_root.exists() {
                    std::fs::remove_dir_all(&paths.data_root)?;
                }
                Ok(())
            })
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("task join: {e}")));
            match result {
                Ok(()) => {
                    let _ = tx.send(AppEvent::Toast("Engine removed".into())).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppEvent::Error(format!("Engine removal failed: {e}")))
                        .await;
                }
            }
            let _ = tx.send(AppEvent::EngineChanged).await;
        });
    });

    dlg.present(Some(&parent));
}

/// Confirm then run a full system prune, reporting reclaimed bytes.
fn confirm_system_prune(state: Rc<AppState>, parent: gtk::Widget) {
    let dlg = adw::AlertDialog::builder()
        .heading("Prune unused Docker data?")
        .body("Unused images, stopped containers, networks and volumes will be deleted.")
        .build();
    dlg.add_response("cancel", "Cancel");
    dlg.add_response("prune", "Prune");
    dlg.set_response_appearance("prune", adw::ResponseAppearance::Destructive);
    dlg.set_default_response(Some("cancel"));
    dlg.set_close_response("cancel");

    dlg.connect_response(None, move |_, response| {
        if response != "prune" {
            return;
        }
        let state = state.clone();
        state.notify(AppEvent::Busy(RefreshScope::Daemon));
        state.with_client(move |client, tx| async move {
            match client.system_prune().await {
                Ok(reclaimed) => {
                    let _ = tx
                        .send(AppEvent::Toast(format!(
                            "Reclaimed {}",
                            format_bytes(reclaimed)
                        )))
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppEvent::Error(format!("system prune failed: {e}")))
                        .await;
                }
            }
            let _ = tx.send(AppEvent::Busy(RefreshScope::Daemon)).await;
        });
    });

    dlg.present(Some(&parent));
}
