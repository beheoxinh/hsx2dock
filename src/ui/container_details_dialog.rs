use std::rc::Rc;

use adw::prelude::*;

use super::app_state::AppState;

/// Modal dialog that shows (tails) a container's logs in a monospace text view.
///
/// Log lines are fetched on the shared Tokio runtime; the buffer is only ever
/// touched from the GTK main loop, via the `main_loop_channel` hand-off.
pub struct ContainerLogsDialog;

impl ContainerLogsDialog {
    pub fn present(state: &Rc<AppState>, container_id: &str) {
        let buffer = gtk::TextBuffer::new(None);
        let view = gtk::TextView::builder()
            .buffer(&buffer)
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(8)
            .right_margin(8)
            .build();

        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .min_content_height(360)
            .min_content_width(720)
            .child(&view)
            .build();

        let tail_combo = gtk::DropDown::from_strings(&["100", "500", "1000", "5000"]);
        tail_combo.set_selected(1);

        let refresh_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Reload logs")
            .build();

        let auto_scroll = gtk::CheckButton::with_label("Follow");
        auto_scroll.set_active(true);

        // Handle of the in-flight follow-stream worker. Aborted whenever the
        // dialog reloads (tail change / refresh) or closes, so at most one
        // stream is attached to the daemon per open dialog.
        let worker: Rc<std::cell::RefCell<Option<tokio::task::JoinHandle<()>>>> =
            Rc::new(std::cell::RefCell::new(None));

        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        header.append(&gtk::Label::new(Some("Lines:")));
        header.append(&tail_combo);
        header.append(&refresh_btn);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        header.append(&spacer);
        header.append(&auto_scroll);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scrolled));

        let dialog = adw::Dialog::builder()
            .title(format!("Logs · {}", short(container_id)))
            .content_width(860)
            .content_height(560)
            .child(&toolbar)
            .build();

        let load: Rc<dyn Fn()> = {
            let state = state.clone();
            let buffer = buffer.clone();
            let view = view.clone();
            let auto_scroll = auto_scroll.clone();
            let tail_combo = tail_combo.clone();
            let worker = worker.clone();
            let container_id = container_id.to_string();
            Rc::new(move || {
                let tail = tail_value(&tail_combo);
                // Stop any previous stream before starting a new one.
                if let Some(handle) = worker.borrow_mut().take() {
                    handle.abort();
                }
                buffer.set_text("Loading logs…\n");
                let follow = auto_scroll.is_active();
                let buffer = buffer.clone();
                let view = view.clone();
                // Incremental appends keep earlier lines on screen while the
                // follow stream is open (`set_text` would wipe the buffer on
                // every chunk). The first message clears the loading banner.
                let append = crate::ui::common::main_loop_channel({
                    let buffer = buffer.clone();
                    let view = view.clone();
                    let first = std::cell::Cell::new(true);
                    move |text: String| {
                        if first.take() {
                            // First chunk replaces the loading banner.
                            buffer.set_text("");
                        }
                        let mut end = buffer.end_iter();
                        buffer.insert(&mut end, &text);
                        if follow {
                            let end = buffer.end_iter();
                            let mark = buffer.create_mark(None, &end, false);
                            view.scroll_mark_onscreen(&mark);
                        }
                    }
                });
                let id = container_id.clone();
                let Some(client) = state.client() else {
                    state.error("Engine is not running");
                    return;
                };
                let tx = state.sender();
                let append_err = append.clone();
                let handle = crate::runtime::runtime().spawn(async move {
                    let result = client
                        .logs_stream(&id, &tail, follow, move |line| {
                            let _ = append.send_blocking(line);
                        })
                        .await;
                    if let Err(e) = result {
                        let _ = append_err.send_blocking(format!("\n[stream ended: {e}]\n"));
                        let _ = tx
                            .send(super::app_state::AppEvent::Error(format!(
                                "container logs: {e}"
                            )))
                            .await;
                    }
                });
                *worker.borrow_mut() = Some(handle);
            })
        };

        let load_for_button = load.clone();
        refresh_btn.connect_clicked(move |_| load_for_button());

        let load_for_combo = load.clone();
        tail_combo.connect_selected_notify(move |_| load_for_combo());

        // Toggling Follow restarts the stream: checked means `follow=true`
        // (live tail), unchecked falls back to a one-shot fetch.
        let load_for_follow = load.clone();
        auto_scroll.connect_toggled(move |_| load_for_follow());

        // Abort the stream when the dialog closes so no worker outlives the
        // buffer it writes to.
        let worker_for_close = worker.clone();
        dialog.connect_closed(move |_| {
            if let Some(handle) = worker_for_close.borrow_mut().take() {
                handle.abort();
            }
        });

        load();
        dialog.present(None::<&gtk::Widget>);
    }
}

fn short(id: &str) -> String {
    id.chars().take(12).collect()
}

fn tail_value(combo: &gtk::DropDown) -> String {
    match combo.selected() {
        0 => "100".to_string(),
        2 => "1000".to_string(),
        3 => "5000".to_string(),
        _ => "500".to_string(),
    }
}
