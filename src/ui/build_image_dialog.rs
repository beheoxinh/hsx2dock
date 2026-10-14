use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::ui::common::main_loop_channel;

/// Modal dialog to build an image from a local build-context directory.
///
/// Streams `docker build` output into a progress bar / status label, mirroring
/// the PullImageDialog progress pattern.
pub struct BuildImageDialog;

impl BuildImageDialog {
    pub fn present(state: &Rc<AppState>) {
        let context_entry = gtk::Entry::builder()
            .placeholder_text("e.g. /home/user/myapp")
            .activates_default(true)
            .hexpand(true)
            .build();
        let dockerfile_entry = gtk::Entry::builder()
            .placeholder_text("Dockerfile")
            .text("Dockerfile")
            .hexpand(true)
            .build();
        let tag_entry = gtk::Entry::builder()
            .placeholder_text("e.g. myapp:v2 (optional)")
            .hexpand(true)
            .build();

        let build_btn = crate::ui::design::button_suggested("Build", None);
        build_btn.set_sensitive(false);

        let progress = gtk::ProgressBar::builder()
            .show_text(true)
            .text("Idle")
            .margin_top(crate::ui::design::SPACE_XS)
            .build();

        let status_label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "caption"])
            .build();

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(crate::ui::design::SPACE_XS)
            .margin_top(crate::ui::design::SPACE_SM)
            .margin_bottom(crate::ui::design::SPACE_SM)
            .margin_start(crate::ui::design::SPACE_SM)
            .margin_end(crate::ui::design::SPACE_SM)
            .build();
        for (label_text, entry) in [
            ("Context directory", &context_entry),
            ("Dockerfile (relative to context)", &dockerfile_entry),
            ("Tag (optional)", &tag_entry),
        ] {
            content.append(
                &gtk::Label::builder()
                    .label(label_text)
                    .xalign(0.0)
                    .css_classes(["caption", "dim-label"])
                    .build(),
            );
            content.append(entry);
        }
        content.append(&progress);
        content.append(&status_label);

        let header = adw::HeaderBar::new();
        header.pack_end(&build_btn);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&content));

        let dialog = adw::Dialog::builder()
            .title("Build Image")
            .content_width(520)
            .child(&toolbar)
            .build();

        let build_btn_for_entry = build_btn.clone();
        context_entry.connect_changed(move |e| {
            build_btn_for_entry.set_sensitive(!e.text().trim().is_empty());
        });

        let state2 = state.clone();
        let dialog_for_click = dialog.clone();
        let context_for_click = context_entry.clone();
        let dockerfile_for_click = dockerfile_entry.clone();
        let tag_for_click = tag_entry.clone();
        build_btn.connect_clicked(move |btn| {
            let context = context_for_click.text().trim().to_string();
            let dockerfile = dockerfile_for_click.text().trim().to_string();
            let tag = tag_for_click.text().trim().to_string();
            if context.is_empty() || !std::path::Path::new(&context).is_dir() {
                status_label.set_label("Context directory must be an existing directory.");
                return;
            }
            btn.set_sensitive(false);
            context_for_click.set_sensitive(false);
            dockerfile_for_click.set_sensitive(false);
            tag_for_click.set_sensitive(false);
            progress.set_fraction(0.0);
            let what = if tag.is_empty() {
                context.clone()
            } else {
                tag.clone()
            };
            status_label.set_label(&format!("Building {what}…"));

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
            let context_path = std::path::PathBuf::from(context);
            st.with_client(move |client, tx| async move {
                let result = client
                    .build_image(&context_path, &dockerfile, &tag, move |msg| {
                        let _ = render_progress.send_blocking(msg);
                    })
                    .await;

                match result {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::Toast(format!("Built {what}"))).await;
                        let _ = tx.send(AppEvent::Busy(RefreshScope::Images)).await;
                        let _ = render_done.send(Ok(())).await;
                        let _ = close.send(()).await;
                    }
                    Err(e) => {
                        let msg = format!("build failed: {e}");
                        let _ = tx.send(AppEvent::Error(msg.clone())).await;
                        let _ = render_done.send(Err(msg)).await;
                    }
                }
            });
        });

        dialog.present(None::<&gtk::Widget>);
    }
}
