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
            .subtitle("Start the bundled engine when the app opens")
            .active(cfg.auto_start_daemon)
            .build();

        let group = adw::PreferencesGroup::builder()
            .title("Engine")
            .description("The app runs its own bundled Docker engine")
            .build();
        group.add(&auto_row);

        let cli_row = adw::SwitchRow::builder()
            .title("Expose docker CLI to the system")
            .subtitle("Create $XDG_RUNTIME_DIR/docker.sock and the docker-manager-engine context")
            .active(cfg.cli_bridge_enabled)
            .build();
        group.add(&cli_row);

        let cli_default_row = adw::SwitchRow::builder()
            .title("Set as default context")
            .subtitle("Run `docker context use docker-manager-engine` so plain `docker` works")
            .active(cfg.cli_bridge_set_default)
            .sensitive(cfg.cli_bridge_enabled)
            .build();
        group.add(&cli_default_row);
        {
            let cli_default_row = cli_default_row.clone();
            cli_row.connect_active_notify(move |s| {
                cli_default_row.set_sensitive(s.is_active());
            });
        }

        let page = adw::PreferencesPage::builder()
            .title("General")
            .icon_name("preferences-system-symbolic")
            .build();
        page.add(&group);

        let save_btn = crate::ui::design::button_suggested("Save", None);
        save_btn.set_tooltip_text(Some("Save preferences"));
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
                compose_file: cfg.compose_file.clone(),
                cli_bridge_enabled: cli_row.is_active(),
                cli_bridge_set_default: cli_default_row.is_active(),
                // Preserve the saved restore target; it is managed by the
                // bridge reconciler, not by this dialog.
                cli_bridge_prev_context: cfg.cli_bridge_prev_context.clone(),
                schema_version: cfg.schema_version,
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
