use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::docker::models::NetworkItem;
use crate::ui::common::copy_to_clipboard;

pub struct NetworkRow {
    root: gtk::Box,
    item: std::cell::RefCell<NetworkItem>,
}

impl NetworkRow {
    fn new(item: NetworkItem, state: Rc<AppState>) -> Rc<Self> {
        let name_label = gtk::Label::builder()
            .label(&item.name)
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        let meta = format!(
            "driver: {} · subnet: {} · gw: {}",
            item.driver, item.subnet, item.gateway
        );
        let meta_label = gtk::Label::builder()
            .label(meta)
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();
        let id_label = gtk::Label::builder()
            .label(item.id.chars().take(14).collect::<String>())
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();

        let delete_btn = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Remove network")
            .css_classes(["destructive-action"])
            .valign(gtk::Align::Center)
            .build();
        let copy_btn = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text("Copy network ID")
            .valign(gtk::Align::Center)
            .build();

        let texts = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        texts.append(&name_label);
        texts.append(&meta_label);
        texts.append(&id_label);

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        root.append(&texts);
        root.append(&copy_btn);
        root.append(&delete_btn);

        let row = Rc::new(Self {
            root,
            item: std::cell::RefCell::new(item),
        });

        let r_copy = row.clone();
        let st_copy = state.clone();
        copy_btn.connect_clicked(move |_| {
            let id = r_copy.item.borrow().id.clone();
            copy_to_clipboard(&r_copy.root.clone().upcast::<gtk::Widget>(), &id);
            st_copy.toast("Copied network ID".to_string());
        });

        let r = row.clone();
        let root_widget = r.root.clone().upcast::<gtk::Widget>();
        let st = state.clone();
        delete_btn.connect_clicked(move |_| {
            let name = r.item.borrow().name.clone();
            let dlg = adw::AlertDialog::builder()
                .heading("Remove network?")
                .body(format!("Network “{name}” will be deleted."))
                .build();
            dlg.add_response("cancel", "Cancel");
            dlg.add_response("remove", "Remove");
            dlg.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            dlg.set_default_response(Some("cancel"));
            let st2 = st.clone();
            dlg.connect_response(None, move |_, resp| {
                if resp != "remove" {
                    return;
                }
                let st3 = st2.clone();
                let name = name.clone();
                st3.with_client(move |client, tx| async move {
                    match client.remove_network(&name).await {
                        Ok(()) => {
                            let _ = tx.send(AppEvent::Toast("Network removed".into())).await;
                            let _ = tx.send(AppEvent::Busy(RefreshScope::Networks)).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(AppEvent::Error(format!("remove network: {e}")))
                                .await;
                        }
                    }
                });
            });
            dlg.present(
                gtk::Widget::root(&root_widget)
                    .and_downcast::<gtk::Window>()
                    .as_ref(),
            );
        });

        row
    }
}

/// Networks management page.
pub struct NetworksView {
    widget: gtk::Box,
    list: gtk::ListBox,
    empty: adw::StatusPage,
    rows: std::cell::RefCell<Vec<Rc<NetworkRow>>>,
    state: Rc<AppState>,
}

impl NetworksView {
    pub fn new(state: Rc<AppState>) -> Rc<Self> {
        let create_btn = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Create network")
            .build();
        let prune_btn = gtk::Button::builder()
            .icon_name("edit-clear-all-symbolic")
            .tooltip_text("Prune unused networks")
            .build();
        let refresh_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Reload networks")
            .build();

        let toolbar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        let title = gtk::Label::builder()
            .label("Networks")
            .xalign(0.0)
            .css_classes(["heading"])
            .hexpand(true)
            .build();
        toolbar.append(&title);
        toolbar.append(&create_btn);
        toolbar.append(&prune_btn);
        toolbar.append(&refresh_btn);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("network-workgroup-symbolic")
            .title("No networks")
            .description("Create a network to connect containers.")
            .build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(
            &adw::Clamp::builder()
                .maximum_size(crate::ui::common::GRID_MAX_WIDTH)
                .child(&list)
                .build(),
            Some("list"),
        );
        stack.add_named(
            &adw::Clamp::builder()
                .maximum_size(crate::ui::common::GRID_MAX_WIDTH)
                .child(&empty)
                .build(),
            Some("empty"),
        );
        stack.set_visible_child_name("empty");

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        widget.append(&crate::ui::common::clamp_grid(&toolbar));
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        widget.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&stack)
                .build(),
        );

        let view = Rc::new(Self {
            widget,
            list,
            empty,
            rows: std::cell::RefCell::new(Vec::new()),
            state,
        });

        let weak = Rc::downgrade(&view);
        refresh_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.refresh();
            }
        });
        let weak = Rc::downgrade(&view);
        create_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                super::dialogs::NetworkCreateDialog::present(&v.state);
            }
        });
        let weak = Rc::downgrade(&view);
        let empty2 = view.empty.clone();
        prune_btn.connect_clicked(move |_| {
            let Some(v) = weak.upgrade() else { return };
            let st = v.state.clone();
            let dlg = adw::AlertDialog::builder()
                .heading("Prune unused networks?")
                .body("Networks not referenced by any container will be deleted.")
                .build();
            dlg.add_response("cancel", "Cancel");
            dlg.add_response("prune", "Prune");
            dlg.set_response_appearance("prune", adw::ResponseAppearance::Destructive);
            dlg.set_default_response(Some("cancel"));
            dlg.connect_response(None, move |_, resp| {
                if resp != "prune" {
                    return;
                }
                let st = st.clone();
                st.with_client(move |client, tx| async move {
                    match client.prune_networks().await {
                        Ok(n) => {
                            let _ = tx
                                .send(AppEvent::Toast(format!("Pruned {n} network(s)")))
                                .await;
                            let _ = tx.send(AppEvent::Busy(RefreshScope::Networks)).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(AppEvent::Error(format!("prune networks: {e}")))
                                .await;
                        }
                    }
                });
            });
            dlg.present(Some(&empty2));
        });

        view.refresh();
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.widget
    }

    pub fn refresh(&self) {
        let st = self.state.clone();
        st.with_client(move |client, tx| async move {
            match client.list_networks().await {
                Ok(items) => {
                    let _ = tx.send(AppEvent::Networks(items)).await;
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("networks: {e}"))).await;
                }
            }
        });
    }

    pub fn set_networks(&self, items: Vec<NetworkItem>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let row = NetworkRow::new(item, self.state.clone());
            self.list.append(&row.root);
            rows.push(row);
        }
        *self.rows.borrow_mut() = rows;
        let is_empty = self.rows.borrow().is_empty();
        if let Some(stack) = self
            .list
            .parent()
            .and_downcast::<adw::Clamp>()
            .and_then(|c| c.parent())
            .and_downcast::<gtk::Stack>()
        {
            stack.set_visible_child_name(if is_empty { "empty" } else { "list" });
        }
    }
}
