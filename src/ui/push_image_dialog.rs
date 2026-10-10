use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::ui::common::main_loop_channel;

/// Modal dialog to push an image, showing live progress from the daemon stream.
pub struct PushImageDialog;

impl PushImageDialog {
    pub fn present(state: &Rc<AppState>, name: &str, anchor: gtk::Widget) {
        let entry = gtk::Entry::builder()
            .placeholder_text("e.g. registry.example.com/myapp:v2")
            .activates_default(true)
            .hexpand(true)
            .text(name)
            .build();

        let push_btn = gtk::Button::with_label("Push");
        push_btn.add_css_class("suggested-action");
        push_btn.set_sensitive(!name.trim().is_empty());

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

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&entry);
        content.append(&progress);
        content.append(&status_label);

        let header = adw::HeaderBar::new();
        header.pack_end(&push_btn);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&content));

        let dialog = adw::Dialog::builder()
            .title("Push Image")
            .content_width(520)
            .child(&toolbar)
            .build();

        let push_btn_for_entry = push_btn.clone();
        entry.connect_changed(move |e| {
            push_btn_for_entry.set_sensitive(!e.text().trim().is_empty());
        });

        let state2 = state.clone();
        let entry_for_click = entry.clone();
        let dialog_for_click = dialog.clone();
        push_btn.connect_clicked(move |btn| {
            let image = entry_for_click.text().trim().to_string();
            if image.is_empty() {
                return;
            }
            btn.set_sensitive(false);
            entry_for_click.set_sensitive(false);
            progress.set_fraction(0.0);
            status_label.set_label(&format!("Pushing {image}…"));

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
                    .push_image(&image_for_worker, move |msg| {
                        let _ = render_progress.send_blocking(msg);
                    })
                    .await;

                match result {
                    Ok(()) => {
                        let _ = tx
                            .send(AppEvent::Toast(format!("Pushed {image_for_worker}")))
                            .await;
                        let _ = tx.send(AppEvent::Busy(RefreshScope::Images)).await;
                        let _ = render_done.send(Ok(())).await;
                        let _ = close.send(()).await;
                    }
                    Err(e) => {
                        let msg = format!("push failed: {e}");
                        let _ = tx.send(AppEvent::Error(msg.clone())).await;
                        let _ = render_done.send(Err(msg)).await;
                    }
                }
            });
        });

        dialog.present(Some(&anchor));
    }
}
