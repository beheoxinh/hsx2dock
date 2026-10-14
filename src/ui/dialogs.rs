use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::Orientation;

use super::app_state::{AppEvent, AppState, RefreshScope};

/// Rich creation dialog for Docker Volumes (Docker Desktop style).
pub struct VolumeCreateDialog;

impl VolumeCreateDialog {
    pub fn present(state: &Rc<AppState>) {
        let name_entry = gtk::Entry::builder()
            .placeholder_text("my-data-volume")
            .activates_default(true)
            .build();

        let driver_combo = gtk::DropDown::from_strings(&["local", "custom"]);
        driver_combo.set_selected(0);

        let custom_driver_entry = gtk::Entry::builder()
            .placeholder_text("e.g. nfs, flocker, weave")
            .visible(false)
            .build();

        let driver_combo_clone = driver_combo.clone();
        let custom_driver_clone = custom_driver_entry.clone();
        driver_combo.connect_selected_notify(move |c| {
            custom_driver_clone.set_visible(c.selected() == 1);
        });

        let driver_opts_entry = gtk::Entry::builder()
            .placeholder_text("type=nfs,o=addr=10.0.0.1,device=:/data")
            .tooltip_text("Comma-separated key=value driver options")
            .build();

        let labels_entry = gtk::Entry::builder()
            .placeholder_text("env=production,project=web")
            .tooltip_text("Comma-separated key=value labels")
            .build();

        let create_btn = crate::ui::design::button_suggested("Create Volume", None);
        create_btn.set_sensitive(false);

        let form = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(crate::ui::design::SPACE_SM)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();

        let group = adw::PreferencesGroup::builder()
            .title("Volume Configuration")
            .description("Specify storage driver and metadata options")
            .build();

        let name_row = adw::ActionRow::builder()
            .title("Volume Name")
            .subtitle("Must be unique on this host")
            .build();
        name_entry.set_valign(gtk::Align::Center);
        name_row.add_suffix(&name_entry);
        group.add(&name_row);

        let driver_box = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(crate::ui::design::SPACE_XS)
            .valign(gtk::Align::Center)
            .build();
        driver_box.append(&driver_combo);
        driver_box.append(&custom_driver_entry);

        let driver_row = adw::ActionRow::builder()
            .title("Driver")
            .subtitle("Storage plugin / driver")
            .build();
        driver_row.add_suffix(&driver_box);
        group.add(&driver_row);

        let opts_row = adw::ActionRow::builder()
            .title("Driver Options")
            .subtitle("Format: key=val,key2=val2")
            .build();
        driver_opts_entry.set_valign(gtk::Align::Center);
        opts_row.add_suffix(&driver_opts_entry);
        group.add(&opts_row);

        let labels_row = adw::ActionRow::builder()
            .title("Labels")
            .subtitle("Format: key=val,key2=val2")
            .build();
        labels_entry.set_valign(gtk::Align::Center);
        labels_row.add_suffix(&labels_entry);
        group.add(&labels_row);

        form.append(&group);

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.pack_end(&create_btn);
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&form));

        let dialog = adw::Dialog::builder()
            .title("Create Volume")
            .content_width(540)
            .child(&toolbar)
            .build();

        let create_btn_for_entry = create_btn.clone();
        name_entry.connect_changed(move |e| {
            create_btn_for_entry.set_sensitive(!e.text().trim().is_empty());
        });

        let state2 = state.clone();
        let name2 = name_entry.clone();
        let driver_combo2 = driver_combo_clone.clone();
        let custom_driver2 = custom_driver_entry.clone();
        let driver_opts2 = driver_opts_entry.clone();
        let labels2 = labels_entry.clone();
        let dialog2 = dialog.clone();

        create_btn.connect_clicked(move |_| {
            let name = name2.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let driver = if driver_combo2.selected() == 1 {
                custom_driver2.text().trim().to_string()
            } else {
                "local".to_string()
            };

            let parse_kv = |raw: &str| -> HashMap<String, String> {
                let mut map = HashMap::new();
                for item in raw.split(',') {
                    if let Some((k, v)) = item.split_once('=') {
                        let k = k.trim().to_string();
                        let v = v.trim().to_string();
                        if !k.is_empty() {
                            map.insert(k, v);
                        }
                    }
                }
                map
            };

            let driver_opts = parse_kv(&driver_opts2.text());
            let labels = parse_kv(&labels2.text());

            let st = state2.clone();
            let dialog = dialog2.clone();
            let close = crate::ui::common::main_loop_channel(move |()| {
                dialog.close();
            });

            st.with_client(move |client, tx| async move {
                match client
                    .create_volume(&name, &driver, driver_opts, labels)
                    .await
                {
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

/// Rich creation dialog for Docker Networks (Docker Desktop style).
pub struct NetworkCreateDialog;

impl NetworkCreateDialog {
    pub fn present(state: &Rc<AppState>) {
        let name_entry = gtk::Entry::builder()
            .placeholder_text("my-network")
            .activates_default(true)
            .build();

        let driver_combo =
            gtk::DropDown::from_strings(&["bridge", "macvlan", "ipvlan", "overlay", "host"]);
        driver_combo.set_selected(0);

        let subnet_entry = gtk::Entry::builder()
            .placeholder_text("172.28.0.0/16")
            .tooltip_text("IP address subnet in CIDR notation")
            .build();

        let gateway_entry = gtk::Entry::builder()
            .placeholder_text("172.28.0.1")
            .tooltip_text("Default gateway IPv4 address")
            .build();

        let internal_switch = gtk::Switch::builder()
            .active(false)
            .valign(gtk::Align::Center)
            .build();

        let attachable_switch = gtk::Switch::builder()
            .active(true)
            .valign(gtk::Align::Center)
            .build();

        let labels_entry = gtk::Entry::builder()
            .placeholder_text("env=staging,app=db")
            .tooltip_text("Comma-separated key=value labels")
            .build();

        let create_btn = crate::ui::design::button_suggested("Create Network", None);
        create_btn.set_sensitive(false);

        let form = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(crate::ui::design::SPACE_SM)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();

        let group = adw::PreferencesGroup::builder()
            .title("Network Configuration")
            .description("Configure IPAM subnet, gateway, driver and flags")
            .build();

        let name_row = adw::ActionRow::builder()
            .title("Network Name")
            .subtitle("Identifier for docker run --network")
            .build();
        name_entry.set_valign(gtk::Align::Center);
        name_row.add_suffix(&name_entry);
        group.add(&name_row);

        let driver_row = adw::ActionRow::builder()
            .title("Driver")
            .subtitle("Network virtualization driver")
            .build();
        driver_combo.set_valign(gtk::Align::Center);
        driver_row.add_suffix(&driver_combo);
        group.add(&driver_row);

        let subnet_row = adw::ActionRow::builder()
            .title("Subnet (CIDR)")
            .subtitle("Optional, e.g. 172.28.0.0/16")
            .build();
        subnet_entry.set_valign(gtk::Align::Center);
        subnet_row.add_suffix(&subnet_entry);
        group.add(&subnet_row);

        let gw_row = adw::ActionRow::builder()
            .title("Gateway")
            .subtitle("Optional, e.g. 172.28.0.1")
            .build();
        gateway_entry.set_valign(gtk::Align::Center);
        gw_row.add_suffix(&gateway_entry);
        group.add(&gw_row);

        let internal_row = adw::ActionRow::builder()
            .title("Internal Network")
            .subtitle("Restrict external network access (isolated)")
            .build();
        internal_row.add_suffix(&internal_switch);
        group.add(&internal_row);

        let attachable_row = adw::ActionRow::builder()
            .title("Attachable")
            .subtitle("Allow manually attached containers")
            .build();
        attachable_row.add_suffix(&attachable_switch);
        group.add(&attachable_row);

        let labels_row = adw::ActionRow::builder()
            .title("Labels")
            .subtitle("Format: key=val,key2=val2")
            .build();
        labels_entry.set_valign(gtk::Align::Center);
        labels_row.add_suffix(&labels_entry);
        group.add(&labels_row);

        form.append(&group);

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.pack_end(&create_btn);
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&form));

        let dialog = adw::Dialog::builder()
            .title("Create Network")
            .content_width(540)
            .child(&toolbar)
            .build();

        let create_btn_for_entry = create_btn.clone();
        name_entry.connect_changed(move |e| {
            create_btn_for_entry.set_sensitive(!e.text().trim().is_empty());
        });

        let state2 = state.clone();
        let name2 = name_entry.clone();
        let driver_combo2 = driver_combo.clone();
        let subnet2 = subnet_entry.clone();
        let gateway2 = gateway_entry.clone();
        let internal2 = internal_switch.clone();
        let attachable2 = attachable_switch.clone();
        let labels2 = labels_entry.clone();
        let dialog2 = dialog.clone();

        create_btn.connect_clicked(move |_| {
            let name = name2.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let driver_idx = driver_combo2.selected();
            let driver = match driver_idx {
                1 => "macvlan",
                2 => "ipvlan",
                3 => "overlay",
                4 => "host",
                _ => "bridge",
            };
            let subnet = subnet2.text().trim().to_string();
            let gateway = gateway2.text().trim().to_string();
            let internal = internal2.is_active();
            let attachable = attachable2.is_active();

            let mut labels = HashMap::new();
            for item in labels2.text().split(',') {
                if let Some((k, v)) = item.split_once('=') {
                    let k = k.trim().to_string();
                    let v = v.trim().to_string();
                    if !k.is_empty() {
                        labels.insert(k, v);
                    }
                }
            }

            let st = state2.clone();
            let dialog = dialog2.clone();
            let close = crate::ui::common::main_loop_channel(move |()| {
                dialog.close();
            });

            st.with_client(move |client, tx| async move {
                match client
                    .create_network(
                        &name, driver, &subnet, &gateway, internal, attachable, labels,
                    )
                    .await
                {
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
