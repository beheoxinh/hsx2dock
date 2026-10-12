use std::rc::Rc;

use adw::prelude::*;
use gtk::{Align, Orientation};

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::docker::models::{ContainerItem, ContainerStats};
use crate::terminal_launcher::TerminalLauncher;
use crate::ui::common::{copy_to_clipboard, format_bytes, format_relative_unix};

/// A single container row with live stats + action buttons.
struct ContainerRow {
    root: gtk::Box,
    status_pill: gtk::Label,
    cpu_bar: gtk::ProgressBar,
    mem_bar: gtk::ProgressBar,
    start_btn: gtk::Button,
    stop_btn: gtk::Button,
    restart_btn: gtk::Button,
    pause_btn: gtk::Button,
    shell_btn: gtk::Button,
    logs_btn: gtk::Button,
    inspect_btn: gtk::Button,
    rename_btn: gtk::Button,
    delete_btn: gtk::Button,
    copy_btn: gtk::Button,
    item: std::cell::RefCell<ContainerItem>,
}

impl ContainerRow {
    fn new(item: ContainerItem, state: Rc<AppState>) -> Rc<Self> {
        let name_label = gtk::Label::builder()
            .label(&item.name)
            .xalign(0.0)
            .css_classes(["heading"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let image_label = gtk::Label::builder()
            .label(&item.image)
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        let id_label = gtk::Label::builder()
            .label(format!(
                "{} · {}",
                item.short_id,
                format_relative_unix(item.created)
            ))
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();
        let status_pill = gtk::Label::builder()
            .label(item.state.to_uppercase())
            .css_classes(["status-pill"])
            .build();

        let cpu_bar = gtk::ProgressBar::builder()
            .show_text(true)
            .text("CPU —")
            .css_classes(["stat-bar"])
            .hexpand(true)
            .build();
        let mem_bar = gtk::ProgressBar::builder()
            .show_text(true)
            .text("MEM —")
            .css_classes(["stat-bar"])
            .hexpand(true)
            .build();

        let start_btn = gtk::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text("Start container")
            .build();
        let stop_btn = gtk::Button::builder()
            .icon_name("media-playback-stop-symbolic")
            .tooltip_text("Stop container")
            .build();
        let restart_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Restart container")
            .build();
        let pause_btn = gtk::Button::builder()
            .icon_name("media-playback-pause-symbolic")
            .tooltip_text("Pause / resume container")
            .build();
        let shell_btn = gtk::Button::builder()
            .icon_name("utilities-terminal-symbolic")
            .tooltip_text("Open shell in terminal (ptyxis/gnome-terminal)")
            .build();
        let logs_btn = gtk::Button::builder()
            .icon_name("text-x-generic-symbolic")
            .tooltip_text("View logs")
            .build();
        let inspect_btn = gtk::Button::builder()
            .icon_name("dialog-information-symbolic")
            .tooltip_text("Inspect container")
            .build();
        let rename_btn = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("Rename container")
            .build();
        let delete_btn = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Remove container")
            .css_classes(["destructive-action"])
            .build();
        let copy_btn = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text("Copy container ID")
            .valign(Align::Center)
            .build();

        let buttons = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .valign(Align::Center)
            .build();
        for b in [
            &start_btn,
            &stop_btn,
            &restart_btn,
            &pause_btn,
            &shell_btn,
            &logs_btn,
            &inspect_btn,
            &rename_btn,
            &delete_btn,
            &copy_btn,
        ] {
            buttons.append(b);
        }

        let title_row = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        title_row.append(&name_label);
        title_row.append(&status_pill);
        let spacer = gtk::Box::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        title_row.append(&spacer);
        title_row.append(&buttons);

        let stats_row = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .build();
        stats_row.append(&cpu_bar);
        stats_row.append(&mem_bar);

        let left = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        left.append(&title_row);
        left.append(&image_label);
        left.append(&id_label);
        left.append(&stats_row);

        let root = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        root.append(&left);

        let row = Rc::new(Self {
            root,
            status_pill,
            cpu_bar,
            mem_bar,
            start_btn,
            stop_btn,
            restart_btn,
            pause_btn,
            shell_btn,
            logs_btn,
            inspect_btn,
            rename_btn,
            delete_btn,
            copy_btn,
            item: std::cell::RefCell::new(item),
        });
        row.bind_actions(state);
        row.sync_buttons();
        row
    }

    fn id(&self) -> String {
        self.item.borrow().id.clone()
    }

    fn bind_actions(self: &Rc<Self>, state: Rc<AppState>) {
        let row = self.clone();
        let st = state.clone();
        self.copy_btn.clone().connect_clicked(move |_| {
            let id = row.id();
            copy_to_clipboard(&row.root.clone().upcast::<gtk::Widget>(), &id);
            st.toast(format!("Copied {}", crate::ui::common::short_id(&id)));
        });
        let row = self.clone();
        let st = state.clone();
        self.start_btn.clone().connect_clicked(move |_| {
            let id = row.id();
            st.with_client(move |client, tx| async move {
                match client.start_container(&id).await {
                    Ok(()) => {
                        let _ = tx
                            .send(AppEvent::Toast(format!("Started {}", short(&id))))
                            .await;
                        let _ = tx.send(AppEvent::Busy(RefreshScope::Containers)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(format!("start: {e}"))).await;
                    }
                }
            });
        });

        let row = self.clone();
        let st = state.clone();
        self.stop_btn.clone().connect_clicked(move |_| {
            let id = row.id();
            st.with_client(move |client, tx| async move {
                match client.stop_container(&id).await {
                    Ok(()) => {
                        let _ = tx
                            .send(AppEvent::Toast(format!("Stopped {}", short(&id))))
                            .await;
                        let _ = tx.send(AppEvent::Busy(RefreshScope::Containers)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(format!("stop: {e}"))).await;
                    }
                }
            });
        });

        let row = self.clone();
        let st = state.clone();
        self.restart_btn.clone().connect_clicked(move |_| {
            let id = row.id();
            st.with_client(move |client, tx| async move {
                match client.restart_container(&id).await {
                    Ok(()) => {
                        let _ = tx
                            .send(AppEvent::Toast(format!("Restarted {}", short(&id))))
                            .await;
                        let _ = tx.send(AppEvent::Busy(RefreshScope::Containers)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(format!("restart: {e}"))).await;
                    }
                }
            });
        });

        let row = self.clone();
        let st = state.clone();
        self.shell_btn.clone().connect_clicked(move |_| {
            let id = row.id();
            if let Err(e) = TerminalLauncher::exec_shell_fallback(&id) {
                st.error(format!("terminal: {e}"));
            } else {
                st.toast("Opened interactive shell");
            }
        });

        let name = self.item.borrow().name.clone();
        let id = self.item.borrow().id.clone();
        let st = state.clone();
        self.logs_btn.clone().connect_clicked(move |_| {
            let _ = name;
            super::container_details_dialog::ContainerLogsDialog::present(&st, &id);
        });

        let row = self.clone();
        let st = state.clone();
        self.pause_btn.clone().connect_clicked(move |_| {
            let id = row.id();
            let paused = row.item.borrow().state.to_lowercase() == "paused";
            st.with_client(move |client, tx| async move {
                let result = if paused {
                    client.unpause_container(&id).await
                } else {
                    client.pause_container(&id).await
                };
                match result {
                    Ok(()) => {
                        let verb = if paused { "Resumed" } else { "Paused" };
                        let _ = tx
                            .send(AppEvent::Toast(format!("{verb} {}", short(&id))))
                            .await;
                        let _ = tx.send(AppEvent::Busy(RefreshScope::Containers)).await;
                    }
                    Err(e) => {
                        let verb = if paused { "resume" } else { "pause" };
                        let _ = tx.send(AppEvent::Error(format!("{verb}: {e}"))).await;
                    }
                }
            });
        });

        let row = self.clone();
        let st = state.clone();
        self.inspect_btn.clone().connect_clicked(move |_| {
            let id = row.id();
            let label = row.item.borrow().name.clone();
            let label_for_show = label.clone();
            st.with_client(move |client, tx| async move {
                match client.inspect_container(&id).await {
                    Ok(d) => {
                        let body = format!(
                            "Name: {}\nImage: {}\nState: {} ({})\nCreated: {}\nCommand: {}\nPorts: {}\nRestart: {}\nNetworks: {}\n\nEnv:\n{}\n\nMounts:\n{}",
                            d.name,
                            d.image,
                            d.state,
                            d.status,
                            d.created,
                            d.command,
                            d.ports,
                            d.restart_policy,
                            d.networks.join(", "),
                            d.env.join("\n"),
                            d.mounts.join("\n"),
                        );
                        let show = crate::ui::common::main_loop_channel(move |()| {
                            let dlg = adw::AlertDialog::builder()
                                .heading(format!("Inspect · {label_for_show}"))
                                .body(body.clone())
                                .build();
                            dlg.add_response("close", "Close");
                            dlg.present(None::<&gtk::Widget>);
                        });
                        let _ = show.send(()).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(format!("inspect: {e}"))).await;
                    }
                }
            });
        });

        let row = self.clone();
        let st = state.clone();
        self.rename_btn.clone().connect_clicked(move |_| {
            let id = row.id();
            let current = row.item.borrow().name.clone();
            let entry = gtk::Entry::builder()
                .text(current.as_str())
                .activates_default(true)
                .build();
            let rename_btn = gtk::Button::with_label("Rename");
            rename_btn.add_css_class("suggested-action");
            let form = gtk::Box::builder()
                .orientation(Orientation::Vertical)
                .spacing(6)
                .margin_top(12)
                .margin_bottom(12)
                .margin_start(12)
                .margin_end(12)
                .build();
            form.append(&gtk::Label::new(Some("New name:")));
            form.append(&entry);
            let toolbar = adw::ToolbarView::new();
            let header = adw::HeaderBar::new();
            header.pack_end(&rename_btn);
            toolbar.add_top_bar(&header);
            toolbar.set_content(Some(&form));
            let dialog = adw::Dialog::builder()
                .title("Rename Container")
                .content_width(400)
                .child(&toolbar)
                .build();
            let st2 = st.clone();
            let entry2 = entry.clone();
            let dialog2 = dialog.clone();
            let id2 = id.clone();
            rename_btn.connect_clicked(move |_| {
                let name = entry2.text().trim().to_string();
                if name.is_empty() {
                    return;
                }
                let st = st2.clone();
                let id = id2.clone();
                let dialog = dialog2.clone();
                let close = crate::ui::common::main_loop_channel(move |()| {
                    dialog.close();
                });
                st.with_client(move |client, tx| async move {
                    match client.rename_container(&id, &name).await {
                        Ok(()) => {
                            let _ = tx.send(AppEvent::Toast(format!("Renamed to {name}"))).await;
                            let _ = tx.send(AppEvent::Busy(RefreshScope::Containers)).await;
                            let _ = close.send(()).await;
                        }
                        Err(e) => {
                            let _ = tx.send(AppEvent::Error(format!("rename: {e}"))).await;
                        }
                    }
                });
            });
            dialog.present(None::<&gtk::Widget>);
        });

        let row = self.clone();
        let st = state.clone();
        self.delete_btn.clone().connect_clicked(move |_| {
            let id = row.id();
            let label = row.item.borrow().name.clone();
            let root_widget = row.root.clone().upcast::<gtk::Widget>();
            let st2 = st.clone();
            let dlg = adw::AlertDialog::builder()
                .heading("Remove container?")
                .body(format!(
                    "This permanently removes “{label}”. Running containers are force-stopped."
                ))
                .build();
            dlg.add_response("cancel", "Cancel");
            dlg.add_response("remove", "Remove");
            dlg.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            dlg.set_default_response(Some("cancel"));
            dlg.connect_response(None, move |_, resp| {
                if resp != "remove" {
                    return;
                }
                let st3 = st2.clone();
                let id = id.clone();
                st3.with_client(move |client, tx| async move {
                    match client.remove_container(&id, true).await {
                        Ok(()) => {
                            let _ = tx.send(AppEvent::Toast("Container removed".into())).await;
                            let _ = tx.send(AppEvent::Busy(RefreshScope::Containers)).await;
                        }
                        Err(e) => {
                            let _ = tx.send(AppEvent::Error(format!("remove: {e}"))).await;
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
    }

    fn sync_buttons(&self) {
        let running = self.item.borrow().running;
        let paused = self.item.borrow().state.to_lowercase() == "paused";
        self.start_btn.set_sensitive(!running);
        self.stop_btn.set_sensitive(running);
        self.restart_btn.set_sensitive(running);
        self.pause_btn.set_sensitive(running);
        self.pause_btn.set_icon_name(if paused {
            "media-playback-start-symbolic"
        } else {
            "media-playback-pause-symbolic"
        });
        self.pause_btn.set_tooltip_text(Some(if paused {
            "Resume container"
        } else {
            "Pause container"
        }));
        self.shell_btn.set_sensitive(running);
        if running {
            self.status_pill.add_css_class("success");
            self.status_pill.remove_css_class("error");
        } else {
            self.status_pill.add_css_class("error");
            self.status_pill.remove_css_class("success");
        }
        if !running {
            self.cpu_bar.set_text(Some("CPU —"));
            self.cpu_bar.set_fraction(0.0);
            self.mem_bar.set_text(Some("MEM —"));
            self.mem_bar.set_fraction(0.0);
        }
    }

    fn set_stats(&self, stats: &ContainerStats) {
        let cpu = stats.cpu_percent.clamp(0.0, 100.0);
        self.cpu_bar.set_fraction(cpu / 100.0);
        self.cpu_bar.set_text(Some(&format!("CPU {cpu:.1}%")));
        let mem = stats.mem_percent().clamp(0.0, 100.0);
        self.mem_bar.set_fraction(mem / 100.0);
        self.mem_bar.set_text(Some(&format!(
            "MEM {} / {} ({mem:.1}%)",
            format_bytes(stats.mem_usage),
            format_bytes(stats.mem_limit),
        )));
    }
}

fn short(id: &str) -> String {
    id.chars().take(12).collect()
}

/// Containers management page: list, filter, live stats and actions.
pub struct ContainersView {
    widget: gtk::Box,
    list: gtk::ListBox,
    search: gtk::SearchEntry,
    empty: adw::StatusPage,
    rows: std::cell::RefCell<Vec<Rc<ContainerRow>>>,
    state: Rc<AppState>,
    show_all: gtk::ToggleButton,
}

impl ContainersView {
    pub fn new(state: Rc<AppState>) -> Rc<Self> {
        use crate::ui::common::{clamp_grid, page_toolbar};

        let search = gtk::SearchEntry::builder()
            .placeholder_text("Filter containers…")
            .hexpand(true)
            .build();

        let show_all = gtk::ToggleButton::builder()
            .label("Show stopped")
            .tooltip_text("Include non-running containers")
            .build();

        let run_btn = gtk::Button::builder()
            .label("Run")
            .icon_name("list-add-symbolic")
            .tooltip_text("Run a new container…")
            .css_classes(["suggested-action"])
            .build();

        // Canonical, clamped toolbar so it lines up with the list below.
        let toolbar = page_toolbar(
            &search,
            &[show_all.clone().upcast(), run_btn.clone().upcast()],
        );

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("applications-system-symbolic")
            .title("No containers")
            .description("Containers created with `docker run` will show up here.")
            .build();

        let list_clamp = clamp_grid(&list);
        let empty_clamp = clamp_grid(&empty);

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(&list_clamp, Some("list"));
        stack.add_named(&empty_clamp, Some("empty"));
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

        let state_for_run = state.clone();
        let view = Rc::new(Self {
            widget,
            list,
            search,
            empty,
            rows: std::cell::RefCell::new(Vec::new()),
            state,
            show_all,
        });

        view.connect_signals();
        run_btn.connect_clicked(move |_| {
            super::run_container_dialog::RunContainerDialog::present(&state_for_run);
        });
        view.refresh();
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.widget
    }

    pub fn refresh(&self) {
        let show_all = self.show_all.is_active();
        let st = self.state.clone();
        st.with_client(move |client, tx| async move {
            match client.list_containers(show_all).await {
                Ok(items) => {
                    let _ = tx.send(AppEvent::Containers(items)).await;
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("containers: {e}"))).await;
                }
            }
        });
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(v) = weak.upgrade() {
                v.apply_filter();
            }
        });
        let weak = Rc::downgrade(self);
        self.show_all.connect_toggled(move |_| {
            if let Some(v) = weak.upgrade() {
                v.refresh();
            }
        });
        let weak = Rc::downgrade(self);
        self.empty.connect_map(move |_| {
            if let Some(v) = weak.upgrade() {
                v.refresh();
            }
        });
    }

    pub fn set_containers(self: &Rc<Self>, items: Vec<ContainerItem>) {
        // Drop old rows, keeping a map by id so stats updates do not lose place.
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let row = ContainerRow::new(item, self.state.clone());
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
            let haystack = format!("{} {} {}", item.name, item.image, item.short_id).to_lowercase();
            let matches = needle.is_empty() || haystack.contains(&needle);
            row.root.set_visible(matches);
            if matches {
                visible += 1;
            }
        }
        let stack = self
            .list
            .parent()
            .and_downcast::<adw::Clamp>()
            .and_then(|c| c.parent())
            .and_downcast::<gtk::Stack>();
        if let Some(stack) = stack {
            stack.set_visible_child_name(if visible == 0 { "empty" } else { "list" });
        }
    }

    /// Apply a stats update coming from the background poller.
    pub fn set_stats(&self, id: &str, stats: &ContainerStats) {
        for row in self.rows.borrow().iter() {
            if row.id() == id {
                row.set_stats(stats);
                break;
            }
        }
    }

    /// Ids of currently running containers (stats poll targets).
    pub fn running_ids(&self) -> Vec<String> {
        self.rows
            .borrow()
            .iter()
            .filter(|r| r.item.borrow().running)
            .map(|r| r.id())
            .collect()
    }
}
