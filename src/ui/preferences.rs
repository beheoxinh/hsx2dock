use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState};
use crate::runtime::spawn;

/// Preferences dialog: daemon unit, preferred socket and auto-start switch.
///
/// Saving persists `~/.config/docker-manager/config.toml` via `AppConfig::save`
/// on a worker thread, then applies the config live on the GTK main loop with
/// `AppState::apply_config` (daemon rebuild + refresh + confirmation toast).
pub struct PreferencesDialog;

impl PreferencesDialog {
    pub fn present(state: &Rc<AppState>) {
        let cfg = state.config.borrow().clone();

        let auto_row = adw::SwitchRow::builder()
            .title("Auto-start daemon")
            .subtitle("Start the systemd unit when no socket is live")
            .active(cfg.auto_start_daemon)
            .build();

        let unit_row = adw::EntryRow::builder().title("Daemon unit").build();
        unit_row.set_text(&cfg.daemon_unit);

        let socket_row = adw::EntryRow::builder().title("Preferred socket").build();
        socket_row.set_text(&cfg.prefer_socket);

        let group = adw::PreferencesGroup::builder()
            .title("Daemon")
            .description("How the app finds and starts the Docker daemon")
            .build();
        group.add(&auto_row);
        // Unit presets: system docker.service first (self-managed Engine),
        // legacy Desktop unit kept for old configs, plus free-form custom.
        let unit_presets = gtk::StringList::new(&[
            "docker.service",
            "docker-desktop.service (legacy)",
            "custom…",
        ]);
        let current = cfg.daemon_unit.trim();
        let preset_row = adw::ComboRow::builder()
            .title("Daemon unit preset")
            .subtitle("System unit this app starts/stops (never Docker Desktop)")
            .model(&unit_presets)
            .build();
        if current == "docker.service" {
            preset_row.set_selected(0);
        } else if current == "docker-desktop.service" {
            preset_row.set_selected(1);
        } else {
            preset_row.set_selected(2);
        }
        group.add(&preset_row);
        group.add(&unit_row);
        group.add(&socket_row);
        // Warn when a legacy Desktop-coupled unit is configured: the app
        // respects it but no longer manages Docker Desktop itself.
        if crate::docker::daemon::is_legacy_desktop_unit(&cfg.daemon_unit) {
            let warn = adw::Banner::builder()
                .title("Legacy Docker Desktop unit configured — switch to docker.service to let this app fully manage the Engine")
                .build();
            warn.add_css_class("warning");
            group.add(&warn);
        }

        let page = adw::PreferencesPage::builder()
            .title("General")
            .icon_name("preferences-system-symbolic")
            .build();
        page.add(&group);

        let save_btn = gtk::Button::builder()
            .label("Save")
            .tooltip_text("Save preferences")
            .build();
        save_btn.add_css_class("suggested-action");

        let header = adw::HeaderBar::new();
        header.pack_end(&save_btn);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        let content = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&page)
            .build();
        toolbar.set_content(Some(&content));

        let dialog = adw::Dialog::builder()
            .title("Preferences")
            .content_width(520)
            .child(&toolbar)
            .build();

        let st = state.clone();
        let dlg = dialog.clone();
        save_btn.connect_clicked(move |_| {
            let prefer_socket =
                match crate::docker::daemon::DaemonManager::validate_preferred_socket(
                    &socket_row.text(),
                ) {
                    Ok(v) => v,
                    Err(msg) => {
                        socket_row.add_css_class("error");
                        socket_row.set_tooltip_text(Some(&msg));
                        return;
                    }
                };
            let next = crate::config::AppConfig {
                auto_start_daemon: auto_row.is_active(),
                daemon_unit: match preset_row.selected() {
                    0 => "docker.service".to_string(),
                    1 => "docker-desktop.service".to_string(),
                    _ => unit_row.text().trim().to_string(),
                },
                prefer_socket,
                compose_file: st.config.borrow().compose_file.clone(),
            };
            let st2 = st.clone();
            let dlg2 = dlg.clone();
            let for_save = next.clone();
            let done =
                crate::ui::common::main_loop_channel(
                    move |result: Result<(), String>| match result {
                        Ok(()) => {
                            st2.apply_config(next.clone());
                            st2.notify(AppEvent::Toast("Preferences saved".into()));
                            dlg2.close();
                        }
                        Err(e) => {
                            st2.notify(AppEvent::Error(format!("save preferences: {e}")));
                        }
                    },
                );
            spawn(async move {
                let out = for_save.save().await.map_err(|e| e.to_string());
                let _ = done.send(out).await;
            });
        });

        dialog.present(None::<&gtk::Widget>);
    }
}
