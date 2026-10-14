//! Animation helpers.
//!
//! Animations respect the GTK setting `gtk-enable-animations`. When the
//! setting is false, we short-circuit to an instant transition so the UI
//! remains accessible to users who prefer reduced motion.
use adw::prelude::*;

fn animations_enabled() -> bool {
    gtk::Settings::default()
        .map(|s| s.is_gtk_enable_animations())
        .unwrap_or(true)
}

fn gentle() -> adw::SpringParams {
    adw::SpringParams::new(0.85, 1.0, 120.0)
}

/// Fade a widget in by animating opacity.
///
/// A cheap, tasteful touch that has almost no readability cost but is
/// visibly more polished. No action is lost if animations are disabled.
pub fn fade_in<W: IsA<gtk::Widget> + Clone + 'static>(widget: &W) {
    if !animations_enabled() {
        widget.set_opacity(1.0);
        return;
    }
    widget.set_opacity(0.0);
    let w = widget.clone();
    let target = adw::CallbackAnimationTarget::new(move |v| w.set_opacity(v));
    let anim = adw::SpringAnimation::new(widget, 0.0, 1.0, gentle(), target);
    anim.set_follow_enable_animations_setting(true);
    anim.play();
}

/// Animates any value from `from` to `to` over a spring, calling `apply` for
/// each frame. Useful for progress bars, percentage labels, etc.
pub fn spring_to<W: IsA<gtk::Widget>>(
    widget: &W,
    from: f64,
    to: f64,
    apply: impl Fn(f64) + 'static,
) {
    let target = adw::CallbackAnimationTarget::new(apply);
    let anim = adw::SpringAnimation::new(widget, from, to, gentle(), target);
    anim.set_follow_enable_animations_setting(true);
    anim.play();
}

/// Convenience wrapper around `adw::SpringAnimation::new` where the target
/// and the widget being animated are the same.
pub fn spring<W: IsA<gtk::Widget> + Clone + 'static>(
    widget: &W,
    from: f64,
    to: f64,
    target: impl IsA<adw::AnimationTarget> + 'static,
) -> adw::SpringAnimation {
    let anim = adw::SpringAnimation::new(widget, from, to, gentle(), target);
    anim.set_follow_enable_animations_setting(true);
    anim
}

/// Revealer helper. If animations are disabled, the transition duration
/// becomes 0 ms so the reveal becomes instantaneous.
pub fn revealer<W: IsA<gtk::Widget>>(
    child: &W,
    transition: gtk::RevealerTransitionType,
) -> gtk::Revealer {
    let duration = if animations_enabled() { 200u32 } else { 0u32 };
    gtk::Revealer::builder()
        .transition_type(transition)
        .transition_duration(duration)
        .child(child)
        .build()
}
