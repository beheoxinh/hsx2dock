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
        group.add(&unit_row);
        group.add(&socket_row);

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
            let next = crate::config::AppConfig {
                auto_start_daemon: auto_row.is_active(),
                daemon_unit: unit_row.text().trim().to_string(),
                prefer_socket: socket_row.text().trim().to_string(),
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
