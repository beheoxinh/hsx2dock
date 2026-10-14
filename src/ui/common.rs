use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_channel::Sender;

/// Bounded bridge from a Tokio worker thread to the GTK main loop.
///
/// GTK widgets are `!Send`, so background futures can never touch them. Create
/// the bridge on the main thread (`f` runs there) and hand the returned `Sender`
/// to the worker: every value sent is delivered to `f` on the main loop.
pub fn main_loop_channel<T, F>(f: F) -> Sender<T>
where
    T: Send + 'static,
    F: Fn(T) + 'static,
{
    let (tx, rx) = async_channel::unbounded::<T>();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(value) = rx.recv().await {
            f(value);
        }
    });
    tx
}

/// Shorten an ID for display (first 12 chars, char-boundary safe).
pub fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

/// Copy `text` to the system clipboard via the widget's display.
pub fn copy_to_clipboard(widget: &gtk::Widget, text: &str) {
    use gtk::prelude::WidgetExt;
    widget.clipboard().set_text(text);
}

/// Human readable byte size (SI-ish, binary units).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Format a `SystemTime` as a relative "x minutes ago" string.
pub fn format_relative(time: SystemTime) -> String {
    let Ok(elapsed) = SystemTime::now().duration_since(time) else {
        return "unknown".to_string();
    };
    format_duration(elapsed)
}

/// Format a unix timestamp (seconds) as a relative string.
pub fn format_relative_unix(secs: i64) -> String {
    if secs <= 0 {
        return "unknown".to_string();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = now.saturating_sub(secs);
    if delta < 0 {
        return "in the future".to_string();
    }
    format_duration(Duration::from_secs(delta as u64))
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..=59 => format!("{secs} seconds ago"),
        60..=3599 => format!("{} minutes ago", secs / 60),
        3600..=86399 => format!("{} hours ago", secs / 3600),
        _ => format!("{} days ago", secs / 86400),
    }
}

// ============================================================================
// Shared UI helpers — one canonical layout so every page lines up.
// ============================================================================

use adw::prelude::*;

/// Maximum content width for a page. Toolbars and lists share this clamp so
/// their left/right gutters align exactly.
pub const CONTENT_MAX_WIDTH: i32 = 1100;

/// Wider width used for dashboard-style pages that carry several columns.
pub const GRID_MAX_WIDTH: i32 = 1400;

/// Wrap `child` in an [`adw::Clamp`] using the shared page width.
pub fn clamp_page<W: IsA<gtk::Widget>>(child: &W) -> adw::Clamp {
    adw::Clamp::builder()
        .maximum_size(CONTENT_MAX_WIDTH)
        .tightening_threshold(600)
        .child(child)
        .build()
}

/// Like [`clamp_page`] but for wide, multi-column dashboard layouts.
pub fn clamp_grid<W: IsA<gtk::Widget>>(child: &W) -> adw::Clamp {
    adw::Clamp::builder()
        .maximum_size(GRID_MAX_WIDTH)
        .tightening_threshold(700)
        .child(child)
        .build()
}

/// A grid of metric tiles that automatically reflows into as many columns as
/// the available width allows (like a responsive card grid).
pub fn metric_grid(items: &[(String, String)]) -> gtk::FlowBox {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(false)
        .row_spacing(12)
        .column_spacing(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    // Keep the tiles from stretching to absurd widths on very wide screens.
    flow.set_max_children_per_line(6);
    flow.set_min_children_per_line(1);
    for (value, caption) in items {
        let tile = metric(value, caption);
        tile.add_css_class("card");
        tile.set_size_request(150, -1);
        flow.insert(&tile, -1);
    }
    flow
}

/// A responsive two-column row. The two children share the width evenly and,
/// because each keeps `hexpand`, they sit side by side when there is room.
/// Callers should place this inside a [`gtk::FlowBox`] or clamp so that it
/// stacks cleanly on narrow windows.
pub fn two_columns(left: &impl IsA<gtk::Widget>, right: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(18)
        .homogeneous(true)
        .valign(gtk::Align::Start)
        .build();
    left.set_hexpand(true);
    right.set_hexpand(true);
    row.append(left);
    row.append(right);
    row
}

/// Left-align a group inside a page so a `PreferencesGroup` does not float in
/// the middle of a wide column.
pub fn group_column<W: IsA<gtk::Widget>>(child: &W) -> gtk::Box {
    let col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk::Align::Start)
        .build();
    col.append(child);
    col
}

/// The canonical page toolbar: a search entry that expands, plus any extra
/// trailing widgets (filters, toggles, icon buttons), all inside one clamp so
/// it shares the list gutter.
pub fn page_toolbar(search: &gtk::SearchEntry, extra: &[gtk::Widget]) -> adw::Clamp {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    search.set_hexpand(true);
    row.append(search);
    for w in extra {
        row.append(w);
    }
    let clamp = clamp_page(&row);
    clamp.add_css_class("page-toolbar");
    clamp
}

/// A bold section heading used inside a view body.
pub fn section_title(text: &str) -> gtk::Label {
    crate::ui::design::section_title(text)
}

/// A big numeric stat with a caption underneath (used on the engine page).
pub fn metric(value: &str, caption: &str) -> gtk::Box {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_halign(gtk::Align::Center);
    let v = gtk::Label::builder()
        .label(value)
        .css_classes(["metric-value"])
        .build();
    let c = gtk::Label::builder()
        .label(caption)
        .css_classes(["metric-caption"])
        .build();
    col.append(&v);
    col.append(&c);
    col
}

/// Like [`metric`] but returns the value label too, so a dashboard can update
/// the number in place. The tile is wrapped in a `.card` for a bit of depth.
pub fn metric_tile(value: &str, caption: &str) -> (gtk::Box, gtk::Label) {
    let value_label = gtk::Label::builder()
        .label(value)
        .css_classes(["metric-value"])
        .build();
    let caption_label = gtk::Label::builder()
        .label(caption)
        .css_classes(["metric-caption"])
        .build();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_halign(gtk::Align::Start);
    col.set_valign(gtk::Align::Start);
    col.add_css_class("card");
    col.set_size_request(150, -1);
    col.append(&value_label);
    col.append(&caption_label);
    (col, value_label)
}

/// Tag a label as a status pill with the matching colour class, replacing any
/// previous tone so the same widget can be re-labelled on state changes.
pub fn set_pill(label: &gtk::Label, text: &str, tone: PillTone) {
    label.set_label(text);
    for t in ["running", "stopped", "warn", "error"] {
        label.remove_css_class(t);
    }
    label.add_css_class(match tone {
        PillTone::Running => "running",
        PillTone::Stopped => "stopped",
        PillTone::Warn => "warn",
        PillTone::Error => "error",
    });
}

/// Colour tones for [`set_pill`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillTone {
    Running,
    Stopped,
    Warn,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_formatting() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.00 KiB");
        assert_eq!(format_bytes(1024 * 1024 * 3 / 2), "1.50 MiB");
    }

    #[test]
    fn bytes_boundaries() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.00 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MiB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.00 GiB");
    }

    #[test]
    fn relative_ranges() {
        let ago = |secs: u64| SystemTime::now() - Duration::from_secs(secs);
        assert_eq!(format_relative(ago(30)), "30 seconds ago");
        assert_eq!(format_relative(ago(90)), "1 minutes ago");
        assert_eq!(format_relative(ago(3700)), "1 hours ago");
        assert_eq!(format_relative(ago(90000)), "1 days ago");
    }

    #[test]
    fn relative_unix_zero_is_unknown() {
        assert_eq!(format_relative_unix(0), "unknown");
    }
}
