//! Docker Manager — a small native GUI for managing Docker resources.
//!
//! Layers:
//! * `docker`  — daemon discovery/lifecycle plus a bollard-backed API client.
//! * `ui`      — libadwaita/GTK4 views; all Docker work happens off the main loop.
//! * `runtime` — the shared Tokio runtime bridging GTK and bollard.

pub mod config;
pub mod docker;
pub mod runtime;
pub mod terminal_launcher;
pub mod ui;
