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
            let container_id = container_id.to_string();
            Rc::new(move || {
                let tail = tail_value(&tail_combo);
                buffer.set_text("Loading logs…\n");
                let follow = auto_scroll.is_active();
                let buffer = buffer.clone();
                let view = view.clone();
                // Widget updates must happen on the main loop: the worker only
                // hands finished text over this channel.
                let render = crate::ui::common::main_loop_channel(move |text: String| {
                    buffer.set_text(&text);
                    if follow {
                        let end = buffer.end_iter();
                        let mark = buffer.create_mark(None, &end, false);
                        view.scroll_mark_onscreen(&mark);
                    }
                });
                let id = container_id.clone();
                state.with_client(move |client, _tx| async move {
                    let result = client.container_logs(&id, &tail).await;
                    let text = match result {
                        Ok(text) => text,
                        Err(e) => format!("Failed to load logs: {e}"),
                    };
                    let _ = render.send(text).await;
                });
            })
        };

        let load_for_button = load.clone();
        refresh_btn.connect_clicked(move |_| load_for_button());

        let load_for_combo = load.clone();
        tail_combo.connect_selected_notify(move |_| load_for_combo());

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
