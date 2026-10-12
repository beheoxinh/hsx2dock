use std::io::Write as _;
use std::rc::Rc;

use adw::prelude::*;

use super::app_state::{AppEvent, AppState};
use crate::runtime::spawn;
use crate::ui::common::main_loop_channel;

/// Modal dialog for `docker login` / `docker logout`.
///
/// The credentials are piped to the `docker login --password-stdin` child
/// process and never stored anywhere in the app: no config file, no memory
/// cache beyond the entry widgets, which are destroyed with the dialog.
pub struct LoginDialog;

impl LoginDialog {
    pub fn present(state: &Rc<AppState>) {
        let registry = gtk::Entry::builder()
            .placeholder_text("Registry (empty = Docker Hub)")
            .hexpand(true)
            .build();
        let username = gtk::Entry::builder()
            .placeholder_text("Username")
            .activates_default(true)
            .hexpand(true)
            .build();
        let password = gtk::PasswordEntry::builder()
            .placeholder_text("Password or access token")
            .activates_default(true)
            .show_peek_icon(true)
            .hexpand(true)
            .build();

        let login_btn = gtk::Button::with_label("Log in");
        login_btn.add_css_class("suggested-action");
        let logout_btn = gtk::Button::with_label("Log out");

        let status_label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "caption"])
            .build();

        let form = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        form.append(&registry);
        form.append(&username);
        form.append(&password);
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::End)
            .build();
        buttons.append(&logout_btn);
        buttons.append(&login_btn);
        form.append(&buttons);
        form.append(&status_label);

        let header = adw::HeaderBar::builder()
            .show_end_title_buttons(false)
            .build();
        header.set_title_widget(Some(&adw::WindowTitle::new("Registry Login", "")));
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&form));

        let dialog = adw::Dialog::builder()
            .title("Registry Login")
            .content_width(480)
            .child(&toolbar)
            .build();

        let tx = state.sender();
        let dialog_for_login = dialog.clone();
        let status_for_login = status_label.clone();
        let registry_for_login = registry.clone();
        let username_for_login = username.clone();
        let password_for_login = password.clone();
        login_btn.connect_clicked(move |_| {
            let server = registry_for_login.text().trim().to_string();
            let user = username_for_login.text().trim().to_string();
            // Take the secret out of the widget once, move it into the worker,
            // then clear the widget so nothing lingers on screen.
            let secret = password_for_login.text().to_string();
            password_for_login.set_text("");
            if user.is_empty() || secret.is_empty() {
                status_for_login.set_label("Username and password are required.");
                return;
            }
            status_for_login.set_label("Logging in…");
            let tx = tx.clone();
            // The dialog is !Send, so it can't cross into the worker. Bridge
            // back to the main loop for the close.
            let close_tx = main_loop_channel({
                let dialog_for_login = dialog_for_login.clone();
                move |_: ()| {
                    dialog_for_login.close();
                }
            });
            let server_for_msg = server.clone();
            let user_for_msg = user.clone();
            spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let mut cmd = crate::docker::engine::docker_command(
                        &crate::docker::engine::EnginePaths::new(),
                    );
                    cmd.arg("login");
                    if !server.is_empty() {
                        cmd.arg(&server);
                    }
                    cmd.arg("--username").arg(&user).arg("--password-stdin");
                    cmd.stdin(std::process::Stdio::piped());
                    cmd.stdout(std::process::Stdio::piped());
                    cmd.stderr(std::process::Stdio::piped());
                    let mut child = match cmd.spawn() {
                        Ok(child) => child,
                        Err(e) => return Err(format!("failed to run docker login: {e}")),
                    };
                    if let Some(mut stdin) = child.stdin.take() {
                        if stdin.write_all(secret.as_bytes()).is_err() {
                            return Err("failed to write password".into());
                        }
                    }
                    match child.wait_with_output() {
                        Ok(out) if out.status.success() => Ok(()),
                        Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                        Err(e) => Err(format!("docker login failed: {e}")),
                    }
                })
                .await;
                let event = match result {
                    Ok(Ok(())) => {
                        let who = if server_for_msg.is_empty() {
                            user_for_msg.clone()
                        } else {
                            format!("{user_for_msg} @ {server_for_msg}")
                        };
                        let _ = close_tx.try_send(());
                        AppEvent::Toast(format!("Logged in as {who}"))
                    }
                    Ok(Err(msg)) => AppEvent::Error(format!("login failed: {msg}")),
                    Err(e) => AppEvent::Error(format!("login task failed: {e}")),
                };
                let _ = tx.send(event).await;
            });
        });

        let tx2 = state.sender();
        let status_for_logout = status_label.clone();
        logout_btn.connect_clicked(move |_| {
            let server = registry.text().trim().to_string();
            status_for_logout.set_label("Logging out…");
            let tx2 = tx2.clone();
            spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let mut cmd = crate::docker::engine::docker_command(
                        &crate::docker::engine::EnginePaths::new(),
                    );
                    cmd.arg("logout");
                    if !server.is_empty() {
                        cmd.arg(&server);
                    }
                    match cmd.output() {
                        Ok(out) if out.status.success() => Ok(()),
                        Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                        Err(e) => Err(format!("docker logout failed: {e}")),
                    }
                })
                .await;
                let event = match result {
                    Ok(Ok(())) => AppEvent::Toast("Logged out".into()),
                    Ok(Err(msg)) => AppEvent::Error(format!("logout failed: {msg}")),
                    Err(e) => AppEvent::Error(format!("logout task failed: {e}")),
                };
                let _ = tx2.send(event).await;
            });
        });

        dialog.present(None::<&gtk::Widget>);
    }
}
