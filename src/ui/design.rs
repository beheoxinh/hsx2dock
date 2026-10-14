//! Design tokens and canonical widget constructors.
//!
//! Every view builds its chrome from the helpers here instead of hand-rolling
//! `gtk::Button::builder()` calls. That is what keeps spacing, tooltips,
//! button styling and status colours uniform across the app — previously each
//! view invented its own margins (4/6/8/12/16 px) and its own button classes.
//!
//! Spatial scale follows the GNOME HIG: a 6 px base unit, with 12 px as the
//! standard gap between related controls and 18/24 px between groups.

use adw::prelude::*;

// ---- Spatial scale ---------------------------------------------------------

/// Gap between tightly related controls (icon + label, pill + text).
pub const SPACE_XS: i32 = 6;
/// Standard gap between controls in a row, and the page gutter.
pub const SPACE_SM: i32 = 12;
/// Gap between groups inside a page.
pub const SPACE_MD: i32 = 18;
/// Gap between major page sections.
pub const SPACE_LG: i32 = 24;

/// Left/right gutter for page content and rows.
pub const MARGIN_PAGE: i32 = 12;
/// Vertical padding inside a list row.
pub const MARGIN_ROW: i32 = 12;

/// Default dialog content size. Dialogs should be a comfortable reading width
/// rather than the grab-bag of 440/480/520/540/560/640 px used before.
pub const DIALOG_WIDTH: i32 = 560;
/// Height for dialogs that carry a scrolling body.
pub const DIALOG_HEIGHT: i32 = 640;

// ---- Buttons ---------------------------------------------------------------

/// The canonical icon-only button.
///
/// Icon-only buttons are always flat + circular and always carry a tooltip —
/// GNOME requires an accessible name, and the tooltip is what supplies it.
pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build()
}

/// Icon button that reads as a positive/primary action.
pub fn icon_button_suggested(icon: &str, tooltip: &str) -> gtk::Button {
    let btn = icon_button(icon, tooltip);
    btn.add_css_class("suggested-action");
    btn
}

/// Icon button for a destructive action (stop, remove).
pub fn icon_button_destructive(icon: &str, tooltip: &str) -> gtk::Button {
    let btn = icon_button(icon, tooltip);
    btn.add_css_class("destructive-action");
    btn
}

/// Text button, optionally with a leading icon.
pub fn button(label: &str, icon: Option<&str>) -> gtk::Button {
    let btn = gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .build();
    if let Some(name) = icon {
        btn.set_icon_name(name);
    }
    btn
}

/// Primary (accent-coloured) text button.
pub fn button_suggested(label: &str, icon: Option<&str>) -> gtk::Button {
    let btn = button(label, icon);
    btn.add_css_class("suggested-action");
    btn
}

/// Text button for a destructive action (stop, remove, down).
pub fn button_destructive(label: &str, icon: Option<&str>) -> gtk::Button {
    let btn = button(label, icon);
    btn.add_css_class("destructive-action");
    btn
}

/// Low-emphasis text button.
pub fn button_flat(label: &str, icon: Option<&str>) -> gtk::Button {
    let btn = button(label, icon);
    btn.add_css_class("flat");
    btn
}

// ---- Status ----------------------------------------------------------------

/// Semantic tone for a status pill. Maps onto `.status-pill.<tone>` in CSS.
pub fn status_tone(state: &str) -> &'static str {
    match state.trim().to_ascii_lowercase().as_str() {
        "running" | "up" | "healthy" | "started" | "online" | "ready" => "ok",
        "paused" | "restarting" | "starting" | "created" | "pending" | "degraded" => "warn",
        "exited" | "stopped" | "dead" | "error" | "removing" | "offline" | "unhealthy" => "err",
        _ => "idle",
    }
}

/// A coloured status badge for a container/daemon state.
///
/// The tone class is what actually colours it; applying only `status-pill`
/// (as the previous code did) left every badge the same neutral grey.
pub fn status_pill(state: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(state.to_uppercase())
        .css_classes(["status-pill", status_tone(state)])
        .valign(gtk::Align::Center)
        .build()
}

/// Re-label and re-tone an existing pill in place (used on refresh).
pub fn retone_pill(pill: &gtk::Label, state: &str) {
    pill.set_label(&state.to_uppercase());
    for tone in ["ok", "warn", "err", "idle"] {
        pill.remove_css_class(tone);
    }
    pill.add_css_class(status_tone(state));
}

// ---- Layout ----------------------------------------------------------------

/// A card: a rounded, subtly-raised surface that groups related rows.
pub fn card() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["card"])
        .build()
}

/// Vertical page body with the standard gutter and inter-group spacing.
pub fn page_body() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(SPACE_MD)
        .margin_top(SPACE_SM)
        .margin_bottom(SPACE_SM)
        .margin_start(MARGIN_PAGE)
        .margin_end(MARGIN_PAGE)
        .build()
}

/// Horizontal row with the standard control gap.
pub fn hrow() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(SPACE_XS)
        .valign(gtk::Align::Center)
        .build()
}

/// Vertical stack with the standard tight gap.
pub fn vstack() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build()
}

/// Wrap a page in a vertical scroller with no horizontal scrollbar and the
/// standard content clamp so toolbars and lists share the same gutters.
pub fn page_scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(child)
        .build()
}

/// Section heading label.
pub fn section_title(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["heading"])
        .margin_top(SPACE_XS)
        .build()
}

/// Secondary/dim caption label.
pub fn caption(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build()
}

/// Monospace label for IDs, commands and paths.
pub fn mono(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .selectable(true)
        .css_classes(["mono"])
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build()
}

#[cfg(test)]
mod tests {
    use super::status_tone;

    #[test]
    fn tone_is_case_and_whitespace_insensitive() {
        // `ContainerItem::state` is the engine's state enum rendered with
        // `{:?}` and lowercased, but it can also arrive from other paths, so
        // the mapping normalises case and surrounding space before matching.
        assert_eq!(status_tone("running"), "ok");
        assert_eq!(status_tone("RUNNING"), "ok");
        assert_eq!(status_tone("  Exited  "), "err");
        assert_eq!(status_tone("Paused"), "warn");
    }

    #[test]
    fn tone_puts_each_state_in_the_right_bucket() {
        for s in ["running", "up", "healthy", "started", "online", "ready"] {
            assert_eq!(status_tone(s), "ok", "{s} should be ok");
        }
        for s in [
            "paused",
            "restarting",
            "starting",
            "created",
            "pending",
            "degraded",
        ] {
            assert_eq!(status_tone(s), "warn", "{s} should be warn");
        }
        for s in [
            "exited",
            "stopped",
            "dead",
            "error",
            "removing",
            "offline",
            "unhealthy",
        ] {
            assert_eq!(status_tone(s), "err", "{s} should be err");
        }
    }

    #[test]
    fn a_stopped_container_is_never_shown_as_ok() {
        // The distinction that matters most to a user: a container that has
        // exited must not read as healthy.
        assert_ne!(status_tone("exited"), "ok");
        assert_ne!(status_tone("stopped"), "ok");
        assert_ne!(status_tone("dead"), "ok");
    }

    #[test]
    fn unknown_states_fall_back_to_the_neutral_tone() {
        assert_eq!(status_tone(""), "idle");
        assert_eq!(status_tone("quantum-superposition"), "idle");
    }
}
