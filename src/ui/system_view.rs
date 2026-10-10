use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::docker::models::{DaemonState, DfReport};
use crate::ui::common::format_bytes;

/// System / Engine information page shown in the main view stack.
pub struct SystemView {
    widget: gtk::Box,
    engine_label: gtk::Label,
    usage_label: gtk::Label,
    guide_box: gtk::Box,
    install_btn: gtk::Button,
    docs_btn: gtk::Button,
    reconfig_btn: gtk::Button,
    uninstall_btn: gtk::Button,
    status_label: gtk::Label,
    daemon_btn: gtk::Button,
    prune_btn: gtk::Button,
    refresh_btn: gtk::Button,
    state: Rc<AppState>,
}

impl SystemView {
    pub fn new(state: Rc<AppState>) -> Rc<Self> {
        let status_label = gtk::Label::builder()
            .label("STOPPED")
            .css_classes(["status-pill"])
            .build();

        let daemon_btn = gtk::Button::with_label("Start Docker");
        daemon_btn.add_css_class("suggested-action");
        let refresh_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh engine information")
            .build();

        let prune_btn = gtk::Button::with_label("System Prune");
        prune_btn.add_css_class("destructive-action");
        prune_btn.set_tooltip_text(Some("Remove unused data (confirm first)"));

        let actions = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .halign(gtk::Align::Center)
            .build();
        actions.append(&daemon_btn);
        actions.append(&refresh_btn);
        actions.append(&prune_btn);

        let engine_label = gtk::Label::builder()
            .label("Waiting for Docker daemon…")
            .selectable(true)
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build();

        let usage_label = gtk::Label::builder()
            .label("Disk usage: —")
            .selectable(true)
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build();

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .valign(gtk::Align::Center)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .build();
        content.append(&status_label);
        content.append(&actions);
        content.append(&engine_label);
        content.append(&usage_label);

        // Install guide panel: visible only when the docker CLI is missing.
        let guide_label = gtk::Label::builder()
            .label("Docker Engine is not installed.\n\n1. Press Install (escalates with your password).\n2. Or follow https://docs.docker.com/engine/install/ manually.\n3. Press Refresh / Reconfig once `docker --version` works.")
            .selectable(true)
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build();
        let install_btn = gtk::Button::with_label("Install Docker Engine");
        install_btn.add_css_class("suggested-action");
        install_btn.set_tooltip_text(Some("Install via distro package manager (pkexec)"));
        let docs_btn = gtk::Button::with_label("Open Install Docs");
        let reconfig_btn = gtk::Button::with_label("Reconfig");
        reconfig_btn.set_tooltip_text(Some("Open Preferences to fix unit/socket"));
        let uninstall_btn = gtk::Button::with_label("Uninstall…");
        uninstall_btn.add_css_class("destructive-action");
        uninstall_btn.set_tooltip_text(Some("Remove Engine packages (confirm first)"));
        let guide_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .halign(gtk::Align::Center)
            .build();
        guide_row.append(&install_btn);
        guide_row.append(&docs_btn);
        guide_row.append(&reconfig_btn);
        guide_row.append(&uninstall_btn);
        let guide_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .halign(gtk::Align::Center)
            .build();
        guide_box.append(&guide_label);
        guide_box.append(&guide_row);
        guide_box.set_visible(false);
        content.append(&guide_box);

        let clamp = adw::Clamp::builder()
            .maximum_size(760)
            .child(&content)
            .build();

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        widget.append(&clamp);

        let view = Rc::new(Self {
            widget,
            engine_label,
            usage_label,
            status_label,
            daemon_btn,
            prune_btn,
            refresh_btn,
            guide_box,
            install_btn,
            docs_btn,
            reconfig_btn,
            uninstall_btn,
            state,
        });

        view.connect_signals();
        view.refresh_daemon_state();
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.widget
    }

    fn connect_signals(self: &Rc<Self>) {
        let state = self.state.clone();
        self.refresh_btn.connect_clicked(move |_| {
            state.refresh_daemon();
        });

        let weak = Rc::downgrade(self);
        self.daemon_btn.connect_clicked(move |btn| {
            let Some(view) = weak.upgrade() else {
                return;
            };
            let state = view.state.clone();
            match state.daemon_state() {
                DaemonState::NotInstalled => {
                    view.install_engine();
                    return;
                }
                DaemonState::Stopped | DaemonState::Error(_) => {
                    btn.set_label("Starting…");
                    state.notify(AppEvent::DaemonState(DaemonState::Starting));
                }
                _ => {
                    btn.set_label("Stopping…");
                }
            }
            btn.set_sensitive(false);
            state.toggle_daemon();
        });

        let weak = Rc::downgrade(self);
        self.install_btn.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.install_engine();
            }
        });
        let weak = Rc::downgrade(self);
        self.docs_btn.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.open_docs();
            }
        });
        let weak = Rc::downgrade(self);
        self.reconfig_btn.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                crate::ui::preferences::PreferencesDialog::present(&view.state);
            }
        });
        let weak = Rc::downgrade(self);
        self.uninstall_btn.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.confirm_uninstall();
            }
        });

        let weak = Rc::downgrade(self);
        self.prune_btn.connect_clicked(move |_| {
            let Some(view) = weak.upgrade() else {
                return;
            };
            view.confirm_system_prune();
        });
    }

    /// Apply the daemon state to the header widgets.
    pub fn refresh_daemon_state(&self) {
        match self.state.daemon_state() {
            DaemonState::Running => {
                self.status_label.set_label("RUNNING");
                self.status_label.remove_css_class("error");
                self.status_label.add_css_class("success");
                self.daemon_btn.set_label("Stop Docker");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("suggested-action");
                self.daemon_btn.add_css_class("destructive-action");
            }
            DaemonState::Starting => {
                self.status_label.set_label("STARTING");
                self.status_label.remove_css_class("error");
                self.status_label.remove_css_class("success");
                self.daemon_btn.set_label("Starting…");
                self.daemon_btn.set_sensitive(false);
            }
            DaemonState::Stopped => {
                self.status_label.set_label("STOPPED");
                self.status_label.remove_css_class("success");
                self.status_label.add_css_class("error");
                self.daemon_btn.set_label("Start Docker");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("destructive-action");
                self.daemon_btn.add_css_class("suggested-action");
                self.engine_label
                    .set_label("Docker daemon is not detected. Start it with the button above.");
            }
            DaemonState::NotInstalled => {
                self.status_label.set_label("NOT INSTALLED");
                self.status_label.remove_css_class("success");
                self.status_label.add_css_class("error");
                self.daemon_btn.set_label("Install Docker");
                self.daemon_btn.set_sensitive(false);
                self.daemon_btn.remove_css_class("destructive-action");
                self.daemon_btn.add_css_class("suggested-action");
                self.engine_label.set_label(
                    "Docker CLI is not installed. See https://docs.docker.com/engine/install/ to install it, then press Refresh.",
                );
            }
            DaemonState::Error(msg) => {
                let msg = msg.clone();
                self.status_label.set_label("ERROR");
                self.status_label.remove_css_class("success");
                self.status_label.add_css_class("error");
                self.daemon_btn.set_label("Start Docker");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("destructive-action");
                self.daemon_btn.add_css_class("suggested-action");
                self.engine_label.set_label(msg.as_str());
            }
        }
        self.guide_box
            .set_visible(matches!(self.state.daemon_state(), DaemonState::NotInstalled));
    }

    /// Run the distro Engine install under pkexec (blocking worker thread).
    fn install_engine(self: &Rc<Self>) {
        self.state.notify(AppEvent::Toast("Installing Docker Engine…".into()));
        let tx = self.state.sender();
        crate::runtime::spawn(async move {
            let ok = tokio::task::spawn_blocking(crate::docker::install::install_blocking)
                .await
                .unwrap_or(Ok(false))
                .unwrap_or(false);
            if ok {
                let _ = tx.send(AppEvent::Toast("Docker Engine installed".into())).await;
            } else {
                let _ = tx
                    .send(AppEvent::Error(
                    "Engine install failed — see https://docs.docker.com/engine/install/".into(),
                ))
                .await;
            }
            let _ = tx.send(AppEvent::EngineChanged).await;
        });
    }

    /// Confirm then uninstall Engine packages, optionally clearing all data.
    fn confirm_uninstall(self: &Rc<Self>) {
        let dlg = adw::AlertDialog::builder()
            .heading("Uninstall Docker Engine?")
            .body("Removes Engine packages via your distro package manager (pkexec). \"Clear data\" also runs `docker system prune -af --volumes`.")
            .build();
        dlg.add_response("cancel", "Cancel");
        dlg.add_response("keep", "Uninstall");
        dlg.add_response("clear", "Uninstall + Clear Data");
        dlg.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        dlg.set_default_response(Some("cancel"));
        dlg.set_close_response("cancel");

        let state = self.state.clone();
        dlg.connect_response(None, move |_, response| {
            if response == "cancel" {
                return;
            }
            let clear_data = response == "clear";
            let tx = state.sender();
            state.notify(AppEvent::Toast("Uninstalling Docker Engine…".into()));
            crate::runtime::spawn(async move {
                let ok = tokio::task::spawn_blocking(move || {
                    crate::docker::install::uninstall_blocking(clear_data)
                })
                .await
                .unwrap_or(Ok(false))
                .unwrap_or(false);
                if ok {
                    let _ = tx.send(AppEvent::Toast("Docker Engine uninstalled".into())).await;
                } else {
                    let _ = tx
                        .send(AppEvent::Error(
                        "Engine uninstall failed — see https://docs.docker.com/engine/install/".into(),
                    ))
                    .await;
                }
                let _ = tx.send(AppEvent::EngineChanged).await;
            });
        });

        if let Some(root) = self.widget.root() {
            dlg.present(Some(&root));
        } else {
            dlg.present(None::<&gtk::Widget>);
        }
    }

    /// Open the official Engine install docs in the default browser.
    fn open_docs(&self) {
        let _ = std::process::Command::new("xdg-open")
            .arg(crate::docker::install::DOCKER_INSTALL_DOCS_URL)
            .spawn();
    }

    /// Render engine details reported by the daemon.
    pub fn set_engine_info(&self, info: &crate::docker::models::EngineInfo, socket: Option<&str>) {
        let text = format!(
            "Engine version: {version}\nAPI version: {api}\nOS: {os} ({arch})\nKernel: {kernel}\n\
             Containers: {running} running · {paused} paused · {stopped} stopped\nImages: {images}\n\
             Memory: {mem} · CPUs: {cpu}\nStorage driver: {driver}\nRoot dir: {root}\nCgroup driver: {cgroup}\nSocket: {socket}",
            version = info.version,
            api = info.api_version,
            os = info.os,
            arch = info.arch,
            kernel = info.kernel,
            running = info.containers_running,
            paused = info.containers_paused,
            stopped = info.containers_stopped,
            images = info.images,
            mem = format_bytes(info.mem_total.max(0) as u64),
            cpu = info.n_cpu,
            driver = info.storage_driver,
            root = info.root_dir,
            cgroup = info.cgroup_driver,
            socket = socket.unwrap_or("default"),
        );
        self.engine_label.set_label(&text);
    }

    /// Render disk usage reported by `docker system df`.
    pub fn set_df(&self, report: &DfReport) {
        let text = format!(
            "Disk usage — Images: {} ({}) · Containers: {} ({}) · Volumes: {} ({})",
            report.images_count,
            format_bytes(report.images_size),
            report.containers_count,
            format_bytes(report.containers_size),
            report.volumes_count,
            format_bytes(report.volumes_size),
        );
        self.usage_label.set_label(&text);
    }

    /// Confirm then run a full system prune, reporting reclaimed bytes.
    fn confirm_system_prune(self: &Rc<Self>) {
        let dlg = adw::AlertDialog::builder()
            .heading("Prune unused Docker data?")
            .body("Unused images, stopped containers, networks and volumes will be deleted.")
            .build();
        dlg.add_response("cancel", "Cancel");
        dlg.add_response("prune", "Prune");
        dlg.set_response_appearance("prune", adw::ResponseAppearance::Destructive);
        dlg.set_default_response(Some("cancel"));
        dlg.set_close_response("cancel");

        let state = self.state.clone();
        dlg.connect_response(None, move |_, response| {
            if response != "prune" {
                return;
            }
            let state = state.clone();
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

        if let Some(root) = self.widget.root() {
            dlg.present(Some(&root));
        } else {
            dlg.present(None::<&gtk::Widget>);
        }
    }
}
