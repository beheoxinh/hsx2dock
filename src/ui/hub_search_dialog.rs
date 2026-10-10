use std::rc::Rc;

use adw::prelude::*;

use super::app_state::AppState;
use crate::ui::common::main_loop_channel;

/// Modal dialog to search Docker Hub and pull a result through the pull dialog.
pub struct HubSearchDialog;

impl HubSearchDialog {
    pub fn present(state: &Rc<AppState>) {
        let entry = gtk::Entry::builder()
            .placeholder_text("e.g. postgres")
            .activates_default(true)
            .hexpand(true)
            .build();

        let search_btn = gtk::Button::with_label("Search");
        search_btn.add_css_class("suggested-action");
        search_btn.set_sensitive(false);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .vexpand(true)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .min_content_height(300)
            .hexpand(true)
            .vexpand(true)
            .child(&list)
            .build();

        let status_label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "caption"])
            .label("Type a term and press Search.")
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
        content.append(&scrolled);
        content.append(&status_label);

        let header = adw::HeaderBar::new();
        header.pack_end(&search_btn);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&content));

        let dialog = adw::Dialog::builder()
            .title("Search Docker Hub")
            .content_width(560)
            .content_height(480)
            .child(&toolbar)
            .build();

        let search_btn_for_entry = search_btn.clone();
        entry.connect_changed(move |e| {
            search_btn_for_entry.set_sensitive(!e.text().trim().is_empty());
        });

        let state2 = state.clone();
        let dialog_weak = dialog.downgrade();
        search_btn.connect_clicked(move |btn| {
            let term = entry.text().trim().to_string();
            if term.is_empty() {
                return;
            }
            btn.set_sensitive(false);
            entry.set_sensitive(false);
            status_label.set_label(&format!("Searching for “{term}”…"));

            let render_results = main_loop_channel({
                let list = list.clone();
                let status_label = status_label.clone();
                let btn = btn.clone();
                let entry = entry.clone();
                let dialog_weak = dialog_weak.clone();
                let state3 = state2.clone();
                move |result: Result<Vec<(String, String, u64)>, String>| {
                    while let Some(child) = list.first_child() {
                        list.remove(&child);
                    }
                    match result {
                        Ok(entries) => {
                            if entries.is_empty() {
                                status_label.set_label("No results.");
                            } else {
                                status_label.set_label(&format!(
                                    "{} result(s). Select one to pull.",
                                    entries.len()
                                ));
                            }
                            for (name, description, stars) in entries {
                                let row = gtk::ListBoxRow::builder()
                                    .activatable(true)
                                    .selectable(true)
                                    .build();
                                let vbox = gtk::Box::builder()
                                    .orientation(gtk::Orientation::Vertical)
                                    .spacing(2)
                                    .margin_top(6)
                                    .margin_bottom(6)
                                    .margin_start(8)
                                    .margin_end(8)
                                    .build();
                                vbox.append(
                                    &gtk::Label::builder()
                                        .label(format!("{name}  ★ {stars}"))
                                        .xalign(0.0)
                                        .build(),
                                );
                                if !description.is_empty() {
                                    vbox.append(
                                        &gtk::Label::builder()
                                            .label(&description)
                                            .xalign(0.0)
                                            .wrap(true)
                                            .css_classes(["dim-label", "caption"])
                                            .build(),
                                    );
                                }
                                row.set_child(Some(&vbox));
                                let state_pull = state3.clone();
                                let dialog_weak = dialog_weak.clone();
                                let name_pull = name.clone();
                                row.connect_activate(move |_| {
                                    if let Some(dlg) = dialog_weak.upgrade() {
                                        dlg.close();
                                    }
                                    super::pull_image_dialog::PullImageDialog::present_with(
                                        &state_pull,
                                        &name_pull,
                                    );
                                });
                                list.append(&row);
                            }
                        }
                        Err(msg) => {
                            status_label.set_label(&msg);
                        }
                    }
                    btn.set_sensitive(true);
                    entry.set_sensitive(true);
                }
            });

            let st = state2.clone();
            st.with_client(move |client, tx| async move {
                let result = client
                    .search_images(&term)
                    .await
                    .map_err(|e| format!("search failed: {e}"));
                if let Err(msg) = &result {
                    let _ = tx
                        .send(super::app_state::AppEvent::Error(msg.clone()))
                        .await;
                }
                let _ = render_results.send(result).await;
            });
        });

        dialog.present(None::<&gtk::Widget>);
    }
}
