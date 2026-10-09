use std::rc::Rc;

use adw::prelude::*;
use gtk::Orientation;

use super::app_state::{AppEvent, AppState};
use crate::docker::models::VolumeItem;

struct VolumeRow {
    root: gtk::Box,
    item: std::cell::RefCell<VolumeItem>,
}

impl VolumeRow {
    fn new(item: VolumeItem, state: Rc<AppState>) -> Rc<Self> {
        let name_label = gtk::Label::builder()
            .label(&item.name)
            .xalign(0.0)
            .css_classes(["heading"])
            .wrap(true)
            .build();
        let meta_label = gtk::Label::builder()
            .label(format!("driver: {}", item.driver))
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();
        let mount_label = gtk::Label::builder()
            .label(item.mountpoint.clone())
            .xalign(0.0)
            .selectable(true)
            .css_classes(["dim-label", "caption"])
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();

        let delete_btn = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Remove volume")
            .css_classes(["destructive-action"])
            .valign(gtk::Align::Center)
            .build();

        let texts = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        texts.append(&name_label);
        texts.append(&meta_label);
        texts.append(&mount_label);

        let root = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        root.append(&texts);
        root.append(&delete_btn);

        let row = Rc::new(Self {
            root,
            item: std::cell::RefCell::new(item),
        });

        let r = row.clone();
        let root_widget = r.root.clone().upcast::<gtk::Widget>();
        let st = state.clone();
        delete_btn.connect_clicked(move |_| {
            let name = r.item.borrow().name.clone();
            let dlg = adw::AlertDialog::builder()
                .heading("Remove volume?")
                .body(format!("Volume “{name}” and its data will be deleted."))
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
                    match client.remove_volume(&name, true).await {
                        Ok(()) => {
                            let _ = tx.send(AppEvent::Toast("Volume removed".into())).await;
                            let _ = tx.send(AppEvent::Busy(true)).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(AppEvent::Error(format!("remove volume: {e}")))
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

/// Volumes management page.
pub struct VolumesView {
    widget: gtk::Box,
    list: gtk::ListBox,
    empty: adw::StatusPage,
    rows: std::cell::RefCell<Vec<Rc<VolumeRow>>>,
    state: Rc<AppState>,
}

impl VolumesView {
    pub fn new(state: Rc<AppState>) -> Rc<Self> {
        let create_btn = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Create volume")
            .build();
        let prune_btn = gtk::Button::builder()
            .icon_name("edit-clear-all-symbolic")
            .tooltip_text("Prune unused volumes")
            .build();
        let refresh_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Reload volumes")
            .build();

        let toolbar = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        let title = gtk::Label::builder()
            .label("Volumes")
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
            .icon_name("drive-harddisk-symbolic")
            .title("No volumes")
            .description("Create a volume to persist container data.")
            .build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(
            &adw::Clamp::builder()
                .maximum_size(1100)
                .child(&list)
                .build(),
            Some("list"),
        );
        stack.add_named(
            &adw::Clamp::builder()
                .maximum_size(1100)
                .child(&empty)
                .build(),
            Some("empty"),
        );
        stack.set_visible_child_name("empty");

        let widget = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        widget.append(&toolbar);
        widget.append(&gtk::Separator::new(Orientation::Horizontal));
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
                super::dialogs::VolumeCreateDialog::present(&v.state);
            }
        });
        let weak = Rc::downgrade(&view);
        let empty2 = view.empty.clone();
        prune_btn.connect_clicked(move |_| {
            let Some(v) = weak.upgrade() else { return };
            let st = v.state.clone();
            let dlg = adw::AlertDialog::builder()
                .heading("Prune unused volumes?")
                .body("Volumes not referenced by any container will be deleted.")
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
                    match client.prune_volumes().await {
                        Ok(n) => {
                            let _ = tx
                                .send(AppEvent::Toast(format!("Pruned {n} volume(s)")))
                                .await;
                            let _ = tx.send(AppEvent::Busy(true)).await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(AppEvent::Error(format!("prune volumes: {e}")))
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
            match client.list_volumes().await {
                Ok(items) => {
                    let _ = tx.send(AppEvent::Volumes(items)).await;
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("volumes: {e}"))).await;
                }
            }
        });
    }

    pub fn set_volumes(&self, items: Vec<VolumeItem>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let row = VolumeRow::new(item, self.state.clone());
            self.list.append(&row.root);
            rows.push(row);
        }
        *self.rows.borrow_mut() = rows;
        self.sync_stack();
    }

    fn sync_stack(&self) {
        let visible = !self.rows.borrow().is_empty();
        if let Some(stack) = self
            .list
            .parent()
            .and_downcast::<adw::Clamp>()
            .and_then(|c| c.parent())
            .and_downcast::<gtk::Stack>()
        {
            stack.set_visible_child_name(if visible { "list" } else { "empty" });
        }
    }
}
