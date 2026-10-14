use std::rc::Rc;

use adw::prelude::*;
use gtk::Orientation;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::docker::models::VolumeItem;
use crate::ui::anim;
use crate::ui::common::copy_to_clipboard;
use crate::ui::design::*;

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

        let inspect_btn = icon_button("system-search-symbolic", "Inspect volume details");
        let delete_btn = icon_button_destructive("user-trash-symbolic", "Remove volume");
        let copy_btn = icon_button("edit-copy-symbolic", "Copy volume name");

        let texts = vstack();
        texts.append(&name_label);
        texts.append(&meta_label);
        texts.append(&mount_label);

        let root = hrow();
        root.set_margin_top(MARGIN_ROW);
        root.set_margin_bottom(MARGIN_ROW);
        root.set_margin_start(MARGIN_PAGE);
        root.set_margin_end(MARGIN_PAGE);
        root.append(&texts);
        root.append(&inspect_btn);
        root.append(&copy_btn);
        root.append(&delete_btn);

        let row = Rc::new(Self {
            root,
            item: std::cell::RefCell::new(item),
        });

        let r_insp = row.clone();
        inspect_btn.connect_clicked(move |_| {
            let item = r_insp.item.borrow().clone();
            let created_str = item.created.as_deref().unwrap_or("Unknown");
            let group = adw::PreferencesGroup::builder()
                .title("Volume")
                .description(item.name.clone())
                .build();
            for (title, value) in [
                ("Name", item.name.clone()),
                ("Driver", item.driver.clone()),
                ("Mountpoint", item.mountpoint.clone()),
                ("Created", created_str.to_string()),
            ] {
                let prop = adw::ActionRow::builder().title(title).build();
                prop.add_css_class("property");
                prop.set_subtitle(&value);
                prop.set_subtitle_selectable(true);
                group.add(&prop);
            }
            let body = page_body();
            body.append(&group);
            let toolbar = adw::ToolbarView::new();
            let header = adw::HeaderBar::new();
            toolbar.add_top_bar(&header);
            toolbar.set_content(Some(&body));
            let dialog = adw::Dialog::builder()
                .title(format!("Volume · {}", item.name))
                .content_width(DIALOG_WIDTH)
                .child(&toolbar)
                .build();
            dialog.present(None::<&gtk::Widget>);
        });

        let r_copy = row.clone();
        let st_copy = state.clone();
        copy_btn.connect_clicked(move |_| {
            let name = r_copy.item.borrow().name.clone();
            copy_to_clipboard(&r_copy.root.clone().upcast::<gtk::Widget>(), &name);
            st_copy.toast(format!("Copied {name}"));
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
                            let _ = tx.send(AppEvent::Busy(RefreshScope::Volumes)).await;
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
        let create_btn = icon_button("list-add-symbolic", "Create volume");
        let prune_btn = icon_button("edit-clear-all-symbolic", "Prune unused volumes");
        let refresh_btn = icon_button("view-refresh-symbolic", "Reload volumes");

        let toolbar = hrow();
        toolbar.set_margin_top(SPACE_XS);
        toolbar.set_margin_bottom(SPACE_XS);
        toolbar.set_margin_start(SPACE_XS);
        toolbar.set_margin_end(SPACE_XS);
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
            .margin_top(SPACE_XS)
            .margin_bottom(SPACE_XS)
            .margin_start(SPACE_XS)
            .margin_end(SPACE_XS)
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
            .orientation(Orientation::Vertical)
            .build();
        widget.append(&crate::ui::common::clamp_grid(&toolbar));
        widget.append(&gtk::Separator::new(Orientation::Horizontal));
        widget.append(&page_scroller(&stack));

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
                            let _ = tx.send(AppEvent::Busy(RefreshScope::Volumes)).await;
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
            anim::fade_in(&row.root);
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
