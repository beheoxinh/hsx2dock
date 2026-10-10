use std::rc::Rc;

use adw::prelude::*;
use gtk::Orientation;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::docker::models::RunConfig;
use crate::ui::common::main_loop_channel;

/// Modal "Run container" wizard: image (typed or picked from local tags),
/// name, command override, port / env / volume mappings, restart policy.
pub struct RunContainerDialog;

impl RunContainerDialog {
    pub fn present(state: &Rc<AppState>) {
        let image_entry = gtk::Entry::builder()
            .placeholder_text("Image, e.g. nginx:latest")
            .hexpand(true)
            .activates_default(true)
            .build();
        let image_combo = gtk::DropDown::builder()
            .tooltip_text("Pick from pulled images")
            .build();
        let name_entry = gtk::Entry::builder()
            .placeholder_text("Container name (optional)")
            .activates_default(true)
            .build();
        let cmd_entry = gtk::Entry::builder()
            .placeholder_text("Command override (optional, space-separated)")
            .activates_default(true)
            .build();
        let ports_view = multiline_view(3);
        let env_view = multiline_view(3);
        let volumes_view = multiline_view(2);
        let restart_combo =
            gtk::DropDown::from_strings(&["no", "always", "unless-stopped", "on-failure"]);
        let detach_check = gtk::CheckButton::builder()
            .label("Run in background (detach)")
            .active(true)
            .build();
        let start_check = gtk::CheckButton::builder()
            .label("Start immediately")
            .active(true)
            .build();
        let run_btn = gtk::Button::with_label("Run");
        run_btn.add_css_class("suggested-action");
        run_btn.set_sensitive(false);

        let image_row = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .build();
        image_row.append(&image_entry);
        image_row.append(&image_combo);

        let opts_row = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .build();
        opts_row.append(&gtk::Label::new(Some("Restart:")));
        opts_row.append(&restart_combo);
        opts_row.append(&detach_check);
        opts_row.append(&start_check);

        let form = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        form.append(&gtk::Label::new(Some("Image:")));
        form.append(&image_row);
        form.append(&gtk::Label::new(Some("Name:")));
        form.append(&name_entry);
        form.append(&gtk::Label::new(Some("Command:")));
        form.append(&cmd_entry);
        form.append(&gtk::Label::new(Some(
            "Ports (one HOST:CONTAINER per line):",
        )));
        form.append(&ports_view);
        form.append(&gtk::Label::new(Some(
            "Environment (one KEY=VALUE per line):",
        )));
        form.append(&env_view);
        form.append(&gtk::Label::new(Some(
            "Volumes (one HOST:CONTAINER per line):",
        )));
        form.append(&volumes_view);
        form.append(&opts_row);

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.pack_end(&run_btn);
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&form));

        let dialog = adw::Dialog::builder()
            .title("Run Container")
            .content_width(600)
            .child(&toolbar)
            .build();

        // Run is only meaningful with an image typed in.
        let run_for_entry = run_btn.clone();
        image_entry.connect_changed(move |e| {
            run_for_entry.set_sensitive(!e.text().trim().is_empty());
        });

        // Fill the image picker from locally pulled tags (main-loop safe).
        let combo_for_fill = image_combo.clone();
        let fill_images = main_loop_channel(move |tags: Vec<String>| {
            if !tags.is_empty() {
                combo_for_fill.set_model(Some(&gtk::StringList::new(
                    &tags.iter().map(String::as_str).collect::<Vec<_>>(),
                )));
            }
        });
        state.with_client(move |client, tx| async move {
            match client.list_images().await {
                Ok(images) => {
                    let mut tags: Vec<String> = images
                        .iter()
                        .flat_map(|i| i.tags.clone())
                        .filter(|t| !t.contains("<none>"))
                        .collect();
                    tags.sort();
                    tags.dedup();
                    let _ = fill_images.send(tags).await;
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("images: {e}"))).await;
                }
            }
        });

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
        run_btn.connect_clicked(move |_| {
            let cfg = RunConfig {
                image: image_entry.text().trim().to_string(),
                name: name_entry.text().trim().to_string(),
                command: cmd_entry
                    .text()
                    .split_whitespace()
                    .map(str::to_string)
                    .collect(),
                ports: RunConfig::parse_lines(&view_text(&ports_view)),
                env: RunConfig::parse_lines(&view_text(&env_view)),
                volumes: RunConfig::parse_lines(&view_text(&volumes_view)),
                restart_policy: restart_value(restart_combo.selected()),
                detach: detach_check.is_active(),
            };
            if let Err(msg) = cfg.validate() {
                state2.error(format!("run: {msg}"));
                return;
            }
            let start = start_check.is_active();
            let st = state2.clone();
            let dialog_for_close = dialog_for_run.clone();
            let close = main_loop_channel(move |()| {
                dialog_for_close.close();
            });
            st.with_client(move |client, tx| async move {
                match client.create_container(&cfg, start).await {
                    Ok(id) => {
                        let short: String = id.chars().take(12).collect();
                        let _ = tx
                            .send(AppEvent::Toast(format!("Created container {short}")))
                            .await;
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

fn multiline_view(lines: i32) -> gtk::TextView {
    let view = gtk::TextView::builder()
        .monospace(true)
        .wrap_mode(gtk::WrapMode::Word)
        .build();
    view.set_size_request(-1, 24 * lines);
    view
}

fn view_text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn restart_value(selected: u32) -> String {
    match selected {
        1 => "always",
        2 => "unless-stopped",
        3 => "on-failure",
        _ => "no",
    }
    .to_string()
}
