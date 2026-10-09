use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState};
use crate::ui::common::main_loop_channel;

/// Modal dialog to pull an image, showing live progress from the daemon stream.
pub struct PullImageDialog;

impl PullImageDialog {
    pub fn present(state: &Rc<AppState>) {
        let entry = gtk::Entry::builder()
            .placeholder_text("e.g. alpine:latest, postgres:17")
            .activates_default(true)
            .hexpand(true)
            .build();

        let pull_btn = gtk::Button::with_label("Pull");
        pull_btn.add_css_class("suggested-action");
        pull_btn.set_sensitive(false);

        let progress = gtk::ProgressBar::builder()
            .show_text(true)
            .text("Idle")
            .margin_top(6)
            .build();

        let status_label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "caption"])
            .build();

        let presets = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        for preset in ["alpine:latest", "ubuntu:24.04", "postgres:17", "redis:7"] {
            let btn = gtk::Button::with_label(preset);
            btn.add_css_class("pill");
            let entry2 = entry.clone();
            btn.connect_clicked(move |_| entry2.set_text(preset));
            presets.append(&btn);
        }

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&entry);
        content.append(&presets);
        content.append(&progress);
        content.append(&status_label);

        let header = adw::HeaderBar::new();
        header.pack_end(&pull_btn);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&content));

        let dialog = adw::Dialog::builder()
            .title("Pull Image")
            .content_width(520)
            .child(&toolbar)
            .build();

        let pull_btn_for_entry = pull_btn.clone();
        entry.connect_changed(move |e| {
            pull_btn_for_entry.set_sensitive(!e.text().trim().is_empty());
        });

        let state2 = state.clone();
        let entry_for_click = entry.clone();
        let dialog_for_click = dialog.clone();
        pull_btn.connect_clicked(move |btn| {
            let image = entry_for_click.text().trim().to_string();
            if image.is_empty() {
                return;
            }
            btn.set_sensitive(false);
            entry_for_click.set_sensitive(false);
            progress.set_fraction(0.0);
            status_label.set_label(&format!("Pulling {image}…"));

            // Widget updates are marshalled back to the GTK main loop.
            let render_progress = main_loop_channel({
                let progress = progress.clone();
                let status_label = status_label.clone();
                move |msg: String| {
                    status_label.set_label(&msg);
                    progress.pulse();
                }
            });
            let render_done = main_loop_channel({
                let progress = progress.clone();
                let status_label = status_label.clone();
                move |result: Result<(), String>| match result {
                    Ok(()) => {
                        progress.set_fraction(1.0);
                        status_label.set_label("Done");
                    }
                    Err(msg) => {
                        progress.set_fraction(0.0);
                        status_label.set_label(&msg);
                    }
                }
            });
            let close = main_loop_channel({
                let dialog = dialog_for_click.clone();
                move |()| {
                    dialog.close();
                }
            });

            let st = state2.clone();
            let image_for_worker = image.clone();
            st.with_client(move |client, tx| async move {
                let result = client
                    .pull_image(&image_for_worker, move |msg| {
                        let _ = render_progress.send_blocking(msg);
                    })
                    .await;

                match result {
                    Ok(()) => {
                        let _ = tx
                            .send(AppEvent::Toast(format!("Pulled {image_for_worker}")))
                            .await;
                        let _ = tx.send(AppEvent::Busy(true)).await;
                        let _ = render_done.send(Ok(())).await;
                        let _ = close.send(()).await;
                    }
                    Err(e) => {
                        let msg = format!("pull failed: {e}");
                        let _ = tx.send(AppEvent::Error(msg.clone())).await;
                        let _ = render_done.send(Err(msg)).await;
                    }
                }
            });
        });

        dialog.present(None::<&gtk::Widget>);
    }
}
