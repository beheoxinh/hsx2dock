//! Engine start/stop progress UI: progress bar, spinner and busy buttons.

use super::*;
use crate::docker::cli_bridge::{self as engine_cli, CliLink, CliStatus, CONTEXT_NAME};
use crate::docker::models::DaemonState;
use gtk::glib;
use std::rc::Rc;

impl SystemView {
    /// Wire the Docker CLI toggle: persist the flag, then reconcile at once.
    pub(crate) fn connect_cli_toggle(self: &Rc<Self>) {
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
    pub(crate) fn start_cli_watch(self: &Rc<Self>) {
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
                // First take-over: remember the user's previous context so a
                // later teardown restores it instead of clobbering it.
                if let Some(prev) = st.prev_context_saved.clone() {
                    view.state
                        .update_config(|c| c.cli_bridge_prev_context = prev);
                }
                view.render_cli_state(&st);
            }
        });
        crate::runtime::spawn(async move {
            let st = crate::runtime::engine_op(move || {
                let applied = engine_cli::reconcile(
                    &socket,
                    cfg.cli_bridge_enabled,
                    cfg.cli_bridge_set_default,
                    engine_live,
                    &cfg.cli_bridge_prev_context,
                )
                .unwrap_or_default();
                let mut st = engine_cli::status(&socket);
                st.prev_context_saved = applied.prev_context_saved;
                st
            })
            .await;
            if let Ok(st) = st {
                let _ = tx.send(st).await;
            }
        });
    }

    /// Paint the "Docker CLI" row from a fresh status snapshot.
    pub(crate) fn render_cli_state(&self, st: &CliStatus) {
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
        // Reflect the persisted preference, not the live link. The switch must
        // stay settable even while the engine is stopped: the flag is only a
        // preference, and `refresh_cli_state` re-applies it (symlink+context)
        // the moment the engine comes back. Greying it out whenever the engine
        // was down made the toggle impossible to turn on at all.
        if self.cli_toggle.is_active() != bridge_enabled {
            self.cli_toggle.set_active(bridge_enabled);
        }
        self.cli_toggle.set_sensitive(true);
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
                // Progress ticks arrive throttled (only every ~256 KiB), so a
                // spring between ticks smooths the bar instead of fighting it.
                let caption = label.to_string();
                let bar = self.progress_bar.clone();
                let text = self.progress_label.clone();
                let from = bar.fraction();
                let driver = bar.clone();
                crate::ui::anim::spring_to(&driver, from, f, move |v| {
                    bar.set_fraction(v);
                    text.set_label(&format!("{caption} — {}%", (v * 100.0).round() as u32));
                });
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

    /// Show an in-button spinner and busy label while a start/stop runs.
    pub(crate) fn set_power_busy(&self, label: &str) {
        set_button_text(&self.start_stop_btn, label);
        self.start_stop_btn.set_sensitive(false);
        self.action_spinner.set_visible(true);
        self.action_spinner.start();
    }
}
