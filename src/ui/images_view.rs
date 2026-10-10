use std::rc::Rc;

use adw::prelude::*;
use gtk::Orientation;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::docker::models::ImageItem;
use crate::ui::common::{copy_to_clipboard, format_bytes, format_relative_unix};

struct ImageRow {
    root: gtk::Box,
    item: std::cell::RefCell<ImageItem>,
}

impl ImageRow {
    fn new(item: ImageItem, state: Rc<AppState>) -> Rc<Self> {
        let tags = if item.tags.is_empty() {
            "<none>:<none>".to_string()
        } else {
            item.tags.join("\n")
        };
        let tags_label = gtk::Label::builder()
            .label(&tags)
            .xalign(0.0)
            .css_classes(["heading"])
            .wrap(true)
            .build();
        let meta_label = gtk::Label::builder()
            .label(format!(
                "{} · created {}",
                format_bytes(item.size.max(0) as u64),
                format_relative_unix(item.created)
            ))
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();
        let id_label = gtk::Label::builder()
            .label(item.short_id.clone())
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();

        let tag_btn = gtk::Button::builder()
            .icon_name("tag-symbolic")
            .tooltip_text("Tag image")
            .valign(gtk::Align::Center)
            .build();
        let push_btn = gtk::Button::builder()
            .icon_name("cloud-upload-symbolic")
            .tooltip_text("Push image to registry")
            .valign(gtk::Align::Center)
            .build();
        let history_btn = gtk::Button::builder()
            .icon_name("view-list-symbolic")
            .tooltip_text("Show image history")
            .valign(gtk::Align::Center)
            .build();
        let delete_btn = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Remove image")
            .css_classes(["destructive-action"])
            .valign(gtk::Align::Center)
            .build();
        let copy_btn = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text("Copy image ID")
            .valign(gtk::Align::Center)
            .build();

        let texts = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        texts.append(&tags_label);
        texts.append(&meta_label);
        texts.append(&id_label);

        let root = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        root.append(&texts);
        root.append(&history_btn);
        root.append(&tag_btn);
        root.append(&push_btn);
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
            st_copy.toast(format!("Copied {}", crate::ui::common::short_id(&id)));
        });

        // Tag: prompt for `repo:tag`, apply to the first tag (or id).
        let r_tag = row.clone();
        let st_tag = state.clone();
        tag_btn.connect_clicked(move |_| {
            let item = r_tag.item.borrow().clone();
            let source = item
                .tags
                .first()
                .cloned()
                .unwrap_or_else(|| item.id.clone());
            let anchor = r_tag.root.clone().upcast::<gtk::Widget>();
            let entry = gtk::Entry::builder()
                .placeholder_text("repo:tag, e.g. myapp:v2")
                .activates_default(true)
                .text(&source)
                .build();
            let tag_btn_inner = gtk::Button::with_label("Tag");
            tag_btn_inner.add_css_class("suggested-action");
            let form = gtk::Box::builder()
                .orientation(Orientation::Vertical)
                .spacing(6)
                .margin_top(12)
                .margin_bottom(12)
                .margin_start(12)
                .margin_end(12)
                .build();
            form.append(&gtk::Label::new(Some("New tag:")));
            form.append(&entry);
            let toolbar = adw::ToolbarView::new();
            let header = adw::HeaderBar::new();
            header.pack_end(&tag_btn_inner);
            toolbar.add_top_bar(&header);
            toolbar.set_content(Some(&form));
            let dialog = adw::Dialog::builder()
                .title(format!("Tag {source}"))
                .content_width(440)
                .child(&toolbar)
                .build();
            let dialog_for_click = dialog.clone();
            let entry_for_click = entry.clone();
            let st_inner = st_tag.clone();
            tag_btn_inner.connect_clicked(move |_| {
                let raw = entry_for_click.text().trim().to_string();
                let Some((repo, tag)) = raw
                    .split_once(':')
                    .filter(|(r, t)| !r.is_empty() && !t.is_empty())
                else {
                    st_inner.error("Tag must look like repo:tag");
                    return;
                };
                let (repo, tag) = (repo.to_string(), tag.to_string());
                let source = source.clone();
                let close = crate::ui::common::main_loop_channel({
                    let dialog = dialog_for_click.clone();
                    move |()| {
                        dialog.close();
                    }
                });
                st_inner.with_client(move |client, tx| async move {
                    match client.tag_image(&source, &repo, &tag).await {
                        Ok(()) => {
                            let _ = tx
                                .send(AppEvent::Toast(format!("Tagged {source} as {repo}:{tag}")))
                                .await;
                            let _ = tx.send(AppEvent::Busy(RefreshScope::Images)).await;
                            let _ = close.send(()).await;
                        }
                        Err(e) => {
                            let _ = tx.send(AppEvent::Error(format!("tag image: {e}"))).await;
                        }
                    }
                });
            });
            dialog.present(
                gtk::Widget::root(&anchor)
                    .and_downcast::<gtk::Window>()
                    .as_ref(),
            );
        });

        // Push: stream daemon progress into a modal progress dialog.
        let r_push = row.clone();
        let st_push = state.clone();
        push_btn.connect_clicked(move |_| {
            let item = r_push.item.borrow().clone();
            let name = item
                .tags
                .first()
                .cloned()
                .unwrap_or_else(|| item.short_id.clone());
            let anchor = r_push.root.clone().upcast::<gtk::Widget>();
            super::push_image_dialog::PushImageDialog::present(&st_push, &name, anchor);
        });

        // History: show `docker history` layers in a scrollable dialog.
        let r_hist = row.clone();
        let st_hist = state.clone();
        history_btn.connect_clicked(move |_| {
            let item = r_hist.item.borrow().clone();
            let title = item
                .tags
                .first()
                .cloned()
                .unwrap_or_else(|| item.short_id.clone());
            let target = item
                .tags
                .first()
                .cloned()
                .unwrap_or_else(|| item.id.clone());
            // Created on the main thread: every delivered value builds and
            // presents the history dialog on the GTK main loop.
            let show = crate::ui::common::main_loop_channel(
                move |rows: Vec<crate::docker::models::ImageHistoryEntry>| {
                    let list = gtk::ListBox::builder()
                        .selection_mode(gtk::SelectionMode::None)
                        .css_classes(["boxed-list"])
                        .margin_top(6)
                        .margin_bottom(6)
                        .margin_start(6)
                        .margin_end(6)
                        .build();
                    for e in &rows {
                        let created = if e.created > 0 {
                            crate::ui::common::format_relative_unix(e.created)
                        } else {
                            "unknown age".to_string()
                        };
                        let heading = if e.tags.is_empty() {
                            e.short_id.clone()
                        } else {
                            e.tags.join(", ")
                        };
                        let row_box = gtk::Box::builder()
                            .orientation(Orientation::Vertical)
                            .spacing(2)
                            .margin_top(6)
                            .margin_bottom(6)
                            .margin_start(12)
                            .margin_end(12)
                            .build();
                        row_box.append(
                            &gtk::Label::builder()
                                .label(format!(
                                    "{} · {} · {}",
                                    heading,
                                    crate::ui::common::format_bytes(e.size.max(0) as u64),
                                    created
                                ))
                                .xalign(0.0)
                                .css_classes(["heading"])
                                .wrap(true)
                                .build(),
                        );
                        row_box.append(
                            &gtk::Label::builder()
                                .label(e.created_by.clone())
                                .xalign(0.0)
                                .css_classes(["dim-label", "caption"])
                                .wrap(true)
                                .build(),
                        );
                        list.append(&row_box);
                    }
                    let scrolled = gtk::ScrolledWindow::builder()
                        .hscrollbar_policy(gtk::PolicyType::Never)
                        .vexpand(true)
                        .child(&list)
                        .build();
                    scrolled.set_size_request(560, 420);
                    let toolbar = adw::ToolbarView::new();
                    let header = adw::HeaderBar::new();
                    toolbar.add_top_bar(&header);
                    toolbar.set_content(Some(&scrolled));
                    let dialog = adw::Dialog::builder()
                        .title(format!("History — {title}"))
                        .child(&toolbar)
                        .build();
                    dialog.present(None::<&gtk::Widget>);
                },
            );
            let st_inner = st_hist.clone();
            st_inner.with_client(move |client, tx| async move {
                match client.image_history(&target).await {
                    Ok(entries) => {
                        let _ = show.send(entries).await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(AppEvent::Error(format!("image history: {e}")))
                            .await;
                    }
                }
            });
        });

        let r = row.clone();
        let st = state.clone();
        delete_btn.connect_clicked(move |_| {
            let item = r.item.borrow().clone();
            let label = if item.tags.is_empty() {
                item.short_id.clone()
            } else {
                item.tags.join(", ")
            };
            let anchor = r.root.clone().upcast::<gtk::Widget>();
            let st2 = st.clone();
            let dlg = adw::AlertDialog::builder()
                .heading("Remove image?")
                .body(format!("This deletes “{label}” from local storage."))
                .build();
            dlg.add_response("cancel", "Cancel");
            dlg.add_response("remove", "Remove");
            dlg.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            dlg.set_default_response(Some("cancel"));
            let id = item.id.clone();
            dlg.connect_response(None, move |_, resp| {
                if resp != "remove" {
                    return;
                }
                let st3 = st2.clone();
                let id = id.clone();
                st3.with_client(move |client, tx| async move {
                    match client.remove_image(&id, true).await {
                        Ok(()) => {
                            let _ = tx.send(AppEvent::Toast("Image removed".into())).await;
                            let _ = tx.send(AppEvent::Busy(RefreshScope::Images)).await;
                        }
                        Err(e) => {
                            let _ = tx.send(AppEvent::Error(format!("remove image: {e}"))).await;
                        }
                    }
                });
            });
            dlg.present(
                gtk::Widget::root(&anchor)
                    .and_downcast::<gtk::Window>()
                    .as_ref(),
            );
        });

        row
    }
}

/// Images management page: tags, size, age, pull and prune.
pub struct ImagesView {
    widget: gtk::Box,
    list: gtk::ListBox,
    search: gtk::SearchEntry,
    empty: adw::StatusPage,
    rows: std::cell::RefCell<Vec<Rc<ImageRow>>>,
    state: Rc<AppState>,
}

impl ImagesView {
    pub fn new(state: Rc<AppState>) -> Rc<Self> {
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Filter images…")
            .hexpand(true)
            .build();

        let pull_btn = gtk::Button::builder()
            .icon_name("cloud-download-symbolic")
            .tooltip_text("Pull a new image")
            .build();
        let build_btn = gtk::Button::builder()
            .icon_name("package-x-generic-symbolic")
            .tooltip_text("Build an image from a directory")
            .build();
        let hub_btn = gtk::Button::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("Search Docker Hub")
            .build();
        let prune_btn = gtk::Button::builder()
            .icon_name("edit-clear-all-symbolic")
            .tooltip_text("Prune unused images")
            .build();
        let refresh_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Reload images")
            .build();

        let toolbar = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        toolbar.append(&search);
        toolbar.append(&pull_btn);
        toolbar.append(&build_btn);
        toolbar.append(&hub_btn);
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
            .icon_name("image-missing-symbolic")
            .title("No images")
            .description("Pull an image to get started.")
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

        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&stack)
            .build();

        let widget = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        widget.append(&toolbar);
        widget.append(&gtk::Separator::new(Orientation::Horizontal));
        widget.append(&scrolled);

        let view = Rc::new(Self {
            widget,
            list,
            search,
            empty,
            rows: std::cell::RefCell::new(Vec::new()),
            state,
        });

        let weak_search = Rc::downgrade(&view);
        view.search.connect_search_changed(move |_| {
            if let Some(v) = weak_search.upgrade() {
                v.apply_filter();
            }
        });
        let weak_refresh = Rc::downgrade(&view);
        refresh_btn.connect_clicked(move |_| {
            if let Some(v) = weak_refresh.upgrade() {
                v.refresh();
            }
        });
        let weak_pull = Rc::downgrade(&view);
        pull_btn.connect_clicked(move |_| {
            if let Some(v) = weak_pull.upgrade() {
                super::pull_image_dialog::PullImageDialog::present(&v.state);
            }
        });
        let weak_build = Rc::downgrade(&view);
        build_btn.connect_clicked(move |_| {
            if let Some(v) = weak_build.upgrade() {
                super::build_image_dialog::BuildImageDialog::present(&v.state);
            }
        });
        let weak_hub = Rc::downgrade(&view);
        hub_btn.connect_clicked(move |_| {
            if let Some(v) = weak_hub.upgrade() {
                super::hub_search_dialog::HubSearchDialog::present(&v.state);
            }
        });
        let weak_prune = Rc::downgrade(&view);
        let root_widget2 = view.empty.clone();
        prune_btn.connect_clicked(move |_| {
            let Some(v) = weak_prune.upgrade() else {
                return;
            };
            let st = v.state.clone();
            let dlg = adw::AlertDialog::builder()
                .heading("Prune unused images?")
                .body("Dangling and unreferenced images will be deleted.")
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
                    match client.prune_images().await {
                        Ok((count, bytes)) => {
                            let _ = tx
                                .send(AppEvent::Toast(format!(
                                    "Pruned {count} image(s), reclaimed {}",
                                    format_bytes(bytes)
                                )))
                                .await;
                            let _ = tx.send(AppEvent::Busy(RefreshScope::Images)).await;
                        }
                        Err(e) => {
                            let _ = tx.send(AppEvent::Error(format!("prune: {e}"))).await;
                        }
                    }
                });
            });
            dlg.present(Some(&root_widget2));
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
            match client.list_images().await {
                Ok(items) => {
                    let _ = tx.send(AppEvent::Images(items)).await;
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("images: {e}"))).await;
                }
            }
        });
    }

    pub fn set_images(&self, items: Vec<ImageItem>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let row = ImageRow::new(item, self.state.clone());
            self.list.append(&row.root);
            rows.push(row);
        }
        *self.rows.borrow_mut() = rows;
        self.apply_filter();
    }

    fn apply_filter(&self) {
        let needle = self.search.text().to_lowercase();
        let rows = self.rows.borrow();
        let mut visible = 0usize;
        for row in rows.iter() {
            let item = row.item.borrow();
            let haystack =
                format!("{} {} {}", item.tags.join(" "), item.short_id, item.id).to_lowercase();
            let matches = needle.is_empty() || haystack.contains(&needle);
            row.root.set_visible(matches);
            if matches {
                visible += 1;
            }
        }
        if let Some(stack) = self
            .list
            .parent()
            .and_downcast::<adw::Clamp>()
            .and_then(|c| c.parent())
            .and_downcast::<gtk::Stack>()
        {
            stack.set_visible_child_name(if visible == 0 { "empty" } else { "list" });
        }
    }
}
