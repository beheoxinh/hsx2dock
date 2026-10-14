//! Compose project page: pick a compose file, run `up`/`down`, inspect the
//! project's services and read its combined logs.
//!
//! The heavy lifting lives in [`crate::docker::compose`], which shells out
//! to the `docker compose` CLI plugin. This view follows the same shape as
//! [`super::networks_view`]: a toolbar, a services list, a logs pane, and
//! workers that report back through [`AppEvent`] so only the Compose tab
//! reloads (`RefreshScope::Compose`).

use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState, RefreshScope};
use crate::docker::compose::{self, ComposeService};
use crate::runtime::spawn;
use crate::ui::common::main_loop_channel;

/// Compose management page (6th tab).
pub struct ComposeView {
    widget: gtk::Box,
    file_entry: gtk::Entry,
    up_btn: gtk::Button,
    down_btn: gtk::Button,
    list: gtk::ListBox,
    empty: adw::StatusPage,
    logs_buffer: gtk::TextBuffer,
    status_label: gtk::Label,
    state: Rc<AppState>,
}

impl ComposeView {
    pub fn new(state: Rc<AppState>) -> Rc<Self> {
        let file_entry = gtk::Entry::builder()
            .placeholder_text("Path to compose file (e.g. ~/projects/web/docker-compose.yml)")
            .hexpand(true)
            .build();
        let browse_btn =
            crate::ui::design::icon_button("document-open-symbolic", "Choose compose file");
        let up_btn = crate::ui::design::button_suggested("Up", None);
        up_btn.set_tooltip_text(Some("Start the project (compose up -d)"));
        let down_btn = crate::ui::design::button_destructive("Down", None);
        down_btn.set_tooltip_text(Some("Stop the project (compose down)"));
        let refresh_btn =
            crate::ui::design::icon_button("view-refresh-symbolic", "Reload services and logs");

        let picker = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(crate::ui::design::SPACE_XS)
            .margin_top(crate::ui::design::SPACE_XS)
            .margin_start(crate::ui::design::SPACE_XS)
            .margin_end(crate::ui::design::SPACE_XS)
            .build();
        picker.append(&file_entry);
        picker.append(&browse_btn);
        picker.append(&up_btn);
        picker.append(&down_btn);
        picker.append(&refresh_btn);

        let heading = gtk::Label::builder()
            .label("Services")
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("package-x-generic-symbolic")
            .title("No services")
            .description("Pick a compose file and press Up, or refresh to list services.")
            .build();
        let services = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(crate::ui::design::SPACE_XS)
            .margin_start(crate::ui::design::SPACE_XS)
            .margin_end(crate::ui::design::SPACE_XS)
            .hexpand(true)
            .vexpand(true)
            .build();
        services.append(&heading);
        services.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .hexpand(true)
                .vexpand(true)
                .child(&list)
                .build(),
        );
        services.append(&empty);

        let logs_heading = gtk::Label::builder()
            .label("Logs")
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        let logs_buffer = gtk::TextBuffer::new(None);
        let logs_view = gtk::TextView::builder()
            .buffer(&logs_buffer)
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .hexpand(true)
            .vexpand(true)
            .build();
        let logs = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(crate::ui::design::SPACE_XS)
            .margin_start(crate::ui::design::SPACE_XS)
            .margin_end(crate::ui::design::SPACE_XS)
            .margin_bottom(crate::ui::design::SPACE_XS)
            .hexpand(true)
            .vexpand(true)
            .build();
        logs.append(&logs_heading);
        logs.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .hexpand(true)
                .vexpand(true)
                .min_content_height(160)
                .child(&logs_view)
                .build(),
        );

        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&services));
        paned.set_end_child(Some(&logs));
        paned.set_hexpand(true);
        paned.set_vexpand(true);

        let status_label = gtk::Label::builder()
            .label("Compose plugin status unknown")
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .margin_start(crate::ui::design::SPACE_XS)
            .margin_bottom(crate::ui::design::SPACE_XS)
            .build();

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        widget.append(&crate::ui::common::clamp_grid(&picker));
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        widget.append(&paned);
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        widget.append(&status_label);

        let saved = state.config.borrow().compose_file.clone();
        if !saved.trim().is_empty() {
            file_entry.set_text(&saved);
        }

        let view = Rc::new(Self {
            widget,
            file_entry,
            up_btn,
            down_btn,
            list,
            empty,
            logs_buffer,
            status_label,
            state,
        });

        let weak = Rc::downgrade(&view);
        browse_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.choose_file();
            }
        });
        let weak = Rc::downgrade(&view);
        refresh_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.refresh();
            }
        });
        let weak = Rc::downgrade(&view);
        view.up_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.run_lifecycle(true);
            }
        });
        let weak = Rc::downgrade(&view);
        let down_root = view.widget.clone().upcast::<gtk::Widget>();
        view.down_btn.connect_clicked(move |_| {
            let Some(v) = weak.upgrade() else {
                return;
            };
            let inner = v.clone();
            let dlg = adw::AlertDialog::builder()
                .heading("Compose down?")
                .body("This stops and removes the project's containers and networks.")
                .build();
            dlg.add_response("cancel", "Cancel");
            dlg.add_response("down", "Down");
            dlg.set_response_appearance("down", adw::ResponseAppearance::Destructive);
            dlg.set_default_response(Some("cancel"));
            dlg.connect_response(None, move |_, resp| {
                if resp == "down" {
                    inner.run_lifecycle(false);
                }
            });
            dlg.present(Some(&down_root));
        });

        view.refresh();
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.widget
    }

    /// Reload service rows, logs and the plugin-availability banner.
    pub fn refresh(&self) {
        let has_plugin = compose::is_available();
        self.status_label.set_label(if has_plugin {
            "Compose plugin available"
        } else {
            "Compose plugin not found — install the docker compose plugin"
        });
        self.up_btn.set_sensitive(has_plugin);
        self.down_btn.set_sensitive(has_plugin);

        let raw = self.file_entry.text().to_string();
        let file = match compose::resolve_compose_file(&raw) {
            Ok(file) => file,
            Err(e) => {
                // No file yet: clear stale rows instead of erroring on every
                // tab visit, but surface the reason in the logs pane.
                self.set_services(Vec::new());
                if raw.trim().is_empty() {
                    self.set_logs(String::new());
                } else {
                    self.set_logs(format!("{e}"));
                }
                return;
            }
        };

        let st = self.state.clone();
        let list_tx = main_loop_channel::<Vec<ComposeService>, _>({
            let list = self.list.clone();
            let empty = self.empty.clone();
            move |items: Vec<ComposeService>| {
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                for item in &items {
                    list.append(&service_row(item));
                }
                empty.set_visible(items.is_empty());
            }
        });
        let logs_tx = main_loop_channel::<String, _>({
            let buffer = self.logs_buffer.clone();
            move |text: String| {
                buffer.set_text(&text);
            }
        });
        let err_tx = st.sender();
        spawn(async move {
            match compose::ps(&file).await {
                Ok(items) => {
                    let _ = list_tx.send(items).await;
                }
                Err(e) => {
                    let _ = err_tx
                        .send(AppEvent::Error(format!("compose ps: {e:#}")))
                        .await;
                }
            }
            match compose::logs(&file, 200).await {
                Ok(text) => {
                    let _ = logs_tx.send(text).await;
                }
                Err(e) => {
                    let _ = logs_tx.send(format!("logs unavailable: {e:#}")).await;
                }
            }
        });
        // Persist the selection for the next launch.
        self.state.config.borrow_mut().compose_file = raw;
    }

    /// `up=true` runs `compose up -d`, otherwise `compose down`.
    fn run_lifecycle(&self, up: bool) {
        let raw = self.file_entry.text().to_string();
        let file = match compose::resolve_compose_file(&raw) {
            Ok(file) => file,
            Err(e) => {
                self.state.error(format!("{e:#}"));
                return;
            }
        };
        self.up_btn.set_sensitive(false);
        self.down_btn.set_sensitive(false);
        let up_btn = self.up_btn.clone();
        let down_btn = self.down_btn.clone();
        let tx = self.state.sender();
        let done_tx = main_loop_channel::<(), _>(move |()| {
            up_btn.set_sensitive(true);
            down_btn.set_sensitive(true);
        });
        let verb = if up { "up" } else { "down" };
        spawn(async move {
            let result = if up {
                compose::up(&file).await
            } else {
                compose::down(&file).await
            };
            match result {
                Ok(out) => {
                    let msg = if out.trim().is_empty() {
                        format!("compose {verb} finished")
                    } else {
                        let first = out.lines().next().unwrap_or("done");
                        format!("compose {verb}: {first}")
                    };
                    let _ = tx.send(AppEvent::Toast(msg)).await;
                    let _ = tx.send(AppEvent::Busy(RefreshScope::Compose)).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppEvent::Error(format!("compose {verb} failed: {e:#}")))
                        .await;
                }
            }
            let _ = done_tx.send(()).await;
        });
    }

    fn choose_file(&self) {
        let entry = self.file_entry.clone();
        let anchor = self.widget.clone().upcast::<gtk::Widget>();
        let parent = anchor.root().and_downcast::<gtk::Window>();
        let dialog = gtk::FileChooserNative::new(
            Some("Choose compose file"),
            parent.as_ref(),
            gtk::FileChooserAction::Open,
            Some("Open"),
            Some("Cancel"),
        );
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Compose files"));
        filter.add_pattern("docker-compose*.y*ml");
        filter.add_pattern("compose*.y*ml");
        filter.add_pattern("*.y*ml");
        dialog.set_filter(&filter);
        let dialog_ref = dialog.clone();
        dialog.connect_response(move |d, response| {
            if response == gtk::ResponseType::Accept {
                if let Some(path) = d.file().and_then(|f| f.path()) {
                    entry.set_text(&path.to_string_lossy());
                }
            }
            dialog_ref.destroy();
        });
        dialog.show();
    }

    fn set_services(&self, items: Vec<ComposeService>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for item in &items {
            self.list.append(&service_row(item));
        }
        self.empty.set_visible(items.is_empty());
    }

    fn set_logs(&self, text: String) {
        self.logs_buffer.set_text(&text);
    }
}

fn service_row(item: &ComposeService) -> gtk::Box {
    let name_label = gtk::Label::builder()
        .label(&item.service)
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    let state_text = if item.health.is_empty() {
        item.state.clone()
    } else {
        format!("{} ({})", item.state, item.health)
    };
    let meta = format!("{} · {} · {}", item.name, item.image, state_text);
    let meta_label = gtk::Label::builder()
        .label(meta.trim())
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build();
    let id_label = gtk::Label::builder()
        .label(format!(
            "{}  {}",
            item.id.chars().take(12).collect::<String>(),
            item.ports
        ))
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build();

    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    texts.append(&name_label);
    texts.append(&meta_label);
    texts.append(&id_label);

    let dot_color = match item.state.as_str() {
        "running" => "success",
        "exited" | "dead" => "error",
        _ => "warning",
    };
    let dot = gtk::Label::builder()
        .label("●")
        .css_classes([dot_color])
        .valign(gtk::Align::Center)
        .build();

    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(8)
        .margin_end(8)
        .margin_top(crate::ui::design::SPACE_XS)
        .margin_bottom(crate::ui::design::SPACE_XS)
        .build();
    row.append(&dot);
    row.append(&texts);
    row
}
