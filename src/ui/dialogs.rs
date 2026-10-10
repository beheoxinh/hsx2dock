use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState, RefreshScope};

pub struct VolumeCreateDialog;

impl VolumeCreateDialog {
    pub fn present(state: &Rc<AppState>) {
        let name_entry = gtk::Entry::builder()
            .placeholder_text("Volume name")
            .activates_default(true)
            .build();
        let driver_entry = gtk::Entry::builder()
            .placeholder_text("Driver (optional, default: local)")
            .activates_default(true)
            .build();
        let create_btn = gtk::Button::with_label("Create");
        create_btn.add_css_class("suggested-action");
        create_btn.set_sensitive(false);

        let form = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        form.append(&gtk::Label::new(Some("Name:")));
        form.append(&name_entry);
        form.append(&gtk::Label::new(Some("Driver:")));
        form.append(&driver_entry);

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.pack_end(&create_btn);
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&form));

        let dialog = adw::Dialog::builder()
            .title("Create Volume")
            .content_width(480)
            .child(&toolbar)
            .build();

        let create_btn_for_entry = create_btn.clone();
        name_entry.connect_changed(move |e| {
            create_btn_for_entry.set_sensitive(!e.text().trim().is_empty());
        });

        let state2 = state.clone();
        let name2 = name_entry.clone();
        let driver2 = driver_entry.clone();
        let dialog2 = dialog.clone();
        create_btn.connect_clicked(move |_| {
            let name = name2.text().trim().to_string();
            let driver = driver2.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let st = state2.clone();
            let dialog = dialog2.clone();
            let close = crate::ui::common::main_loop_channel(move |()| {
                dialog.close();
            });
            st.with_client(move |client, tx| async move {
                match client.create_volume(&name, &driver).await {
                    Ok(()) => {
                        let _ = tx
                            .send(AppEvent::Toast(format!("Created volume {name}")))
                            .await;
                        let _ = tx.send(AppEvent::Busy(RefreshScope::Volumes)).await;
                        let _ = close.send(()).await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(AppEvent::Error(format!("create volume: {e}")))
                            .await;
                    }
                }
            });
        });

        dialog.present(None::<&gtk::Widget>);
    }
}

pub struct NetworkCreateDialog;

impl NetworkCreateDialog {
    pub fn present(state: &Rc<AppState>) {
        let name_entry = gtk::Entry::builder()
            .placeholder_text("Network name")
            .activates_default(true)
            .build();
        let driver_entry = gtk::Entry::builder()
            .placeholder_text("Driver (optional, default: bridge)")
            .activates_default(true)
            .build();
        let create_btn = gtk::Button::with_label("Create");
        create_btn.add_css_class("suggested-action");
        create_btn.set_sensitive(false);

        let form = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        form.append(&gtk::Label::new(Some("Name:")));
        form.append(&name_entry);
        form.append(&gtk::Label::new(Some("Driver:")));
        form.append(&driver_entry);

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.pack_end(&create_btn);
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&form));

        let dialog = adw::Dialog::builder()
            .title("Create Network")
            .content_width(480)
            .child(&toolbar)
            .build();

        let create_btn_for_entry = create_btn.clone();
        name_entry.connect_changed(move |e| {
            create_btn_for_entry.set_sensitive(!e.text().trim().is_empty());
        });

        let state2 = state.clone();
        let name2 = name_entry.clone();
        let driver2 = driver_entry.clone();
        let dialog2 = dialog.clone();
        create_btn.connect_clicked(move |_| {
            let name = name2.text().trim().to_string();
            let driver = driver2.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let st = state2.clone();
            let dialog = dialog2.clone();
            let close = crate::ui::common::main_loop_channel(move |()| {
                dialog.close();
            });
            st.with_client(move |client, tx| async move {
                match client.create_network(&name, &driver).await {
                    Ok(()) => {
                        let _ = tx
                            .send(AppEvent::Toast(format!("Created network {name}")))
                            .await;
                        let _ = tx.send(AppEvent::Busy(RefreshScope::Networks)).await;
                        let _ = close.send(()).await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(AppEvent::Error(format!("create network: {e}")))
                            .await;
                    }
                }
            });
        });

        dialog.present(None::<&gtk::Widget>);
    }
}
