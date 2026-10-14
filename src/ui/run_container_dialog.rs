use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::docker::models::RunConfig;
use crate::ui::common::main_loop_channel;
use crate::ui::run_form::RunConfigForm;

/// Modal "Run container" wizard: a structured, Docker-Desktop-style form
/// (image, name, ports, environment, volumes, restart policy, resources,
/// labels, capabilities, health check) backed by [`RunConfig`].
pub struct RunContainerDialog;

impl RunContainerDialog {
    /// Open an empty form.
    pub fn present(state: &Rc<AppState>) {
        Self::present_with(state, None, "Run Container");
    }

    /// Open the form pre-filled from `initial`, used to duplicate or recreate
    /// an existing container.
    pub fn present_with(state: &Rc<AppState>, initial: Option<RunConfig>, title: &str) {
        let form = RunConfigForm::new(initial.as_ref());

        let image_entry = form.image_entry().clone();
        let image_combo = gtk::DropDown::builder()
            .tooltip_text("Pick from pulled images")
            .build();

        let image_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(crate::ui::design::SPACE_XS)
            .build();
        image_row.append(&image_entry);
        image_row.append(&image_combo);

        // The form owns the image entry, so the picker sits above it.
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(crate::ui::design::SPACE_XS)
            .build();
        body.append(&image_row);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(form.widget())
            .build();
        body.append(&scrolled);

        let run_btn = crate::ui::design::button_suggested("Run", None);
        run_btn.set_sensitive(false);

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(crate::ui::design::SPACE_SM)
            .margin_end(crate::ui::design::SPACE_SM)
            .build();
        content.append(&body);
        content.append(&run_btn);

        let dialog = adw::Dialog::builder()
            .title(title)
            .content_width(720)
            .content_height(crate::ui::design::DIALOG_HEIGHT)
            .child(&content)
            .build();

        // Run is only meaningful with an image typed in.
        {
            let run_for_entry = run_btn.clone();
            image_entry.connect_changed(move |e| {
                run_for_entry.set_sensitive(!e.text().trim().is_empty());
            });
        }

        // Fill the image picker from locally pulled tags (main-loop safe).
        let combo_for_fill = image_combo.clone();
        let fill_images = main_loop_channel(move |tags: Vec<String>| {
            if !tags.is_empty() {
                combo_for_fill.set_model(Some(&gtk::StringList::new(
                    &tags.iter().map(String::as_str).collect::<Vec<_>>(),
                )));
            }
        });
        {
            let st = state.clone();
            let fill_images_ch = fill_images.clone();
            st.with_client(move |client, _tx| async move {
                if let Ok(images) = client.list_images().await {
                    let mut tags: Vec<String> = images
                        .iter()
                        .flat_map(|i| i.tags.iter().cloned())
                        .filter(|t| !t.contains("<none>"))
                        .collect();
                    tags.sort();
                    tags.dedup();
                    let _ = fill_images_ch.send(tags).await;
                }
            });
        }

        // Picking a tag copies it into the entry for editing.
        let entry_for_combo = image_entry.clone();
        image_combo.connect_selected_notify(move |combo| {
            if let Some(obj) = combo
                .model()
                .and_then(|m| m.item(combo.selected()))
                .and_downcast::<gtk::StringObject>()
            {
                entry_for_combo.set_text(&obj.string());
            }
        });

        let state2 = state.clone();
        let dialog_for_run = dialog.clone();
        let form_for_run = form.clone();
        run_btn.connect_clicked(move |_| {
            let cfg = form_for_run.config();
            if let Err(e) = cfg.validate() {
                state2.error(format!("Run: {e}"));
                return;
            }
            let start = form_for_run.start_immediately();
            let st = state2.clone();
            let dialog_for_close = dialog_for_run.clone();
            let close = main_loop_channel(move |()| {
                dialog_for_close.close();
            });
            st.with_client(move |client, tx| async move {
                match client.create_container(&cfg, start).await {
                    Ok(id) => {
                        let short: String = id.chars().take(12).collect();
                        let _ = tx.send(AppEvent::Toast(format!("Created {short}"))).await;
                        let _ = tx.send(AppEvent::Busy(RefreshScope::Containers)).await;
                        let _ = close.send(()).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(format!("run: {e}"))).await;
                    }
                }
            });
        });

        dialog.present(None::<&gtk::Widget>);
    }
}
