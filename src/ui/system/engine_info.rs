//! Engine info rows (version, API, OS, socket) and daemon-state rendering.

use super::*;
use crate::docker::engine::{self, DEFAULT_RELEASE};
use crate::docker::models::{DaemonState, EngineInfo};
use crate::ui::common::format_bytes;

impl SystemView {
    /// Fill the engine-fact rows from the on-disk engine and the pinned release.
    pub(crate) fn render_engine_info(&self) {
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

    /// Reveal the preflight banner when rootless networking would fail.
    pub(crate) fn check_net_helper(&self) {
        let paths = self.state.engine.paths();
        let missing = self.state.engine.is_installed() && engine::missing_net_helper(paths);
        self.banner.set_revealed(missing);
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
}
