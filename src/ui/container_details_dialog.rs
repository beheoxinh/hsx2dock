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

        let refresh_btn = crate::ui::design::icon_button("view-refresh-symbolic", "Reload logs");

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

/// Detailed Inspector dialog for a container (Docker Desktop / Rancher Desktop style).
pub struct ContainerInspectDialog;

impl ContainerInspectDialog {
    pub fn present(state: &Rc<AppState>, details: crate::docker::models::ContainerDetails) {
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();

        let duplicate_btn =
            crate::ui::design::button_suggested("Edit & Recreate…", Some("document-edit-symbolic"));
        duplicate_btn.add_css_class("suggested-action");
        header.pack_end(&duplicate_btn);
        toolbar.add_top_bar(&header);

        let view_stack = adw::ViewStack::new();

        // 1. Overview Tab
        let overview_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();

        let group = adw::PreferencesGroup::builder()
            .title("General Information")
            .build();

        let add_pref_row = |grp: &adw::PreferencesGroup, title: &str, subtitle: &str| {
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(subtitle)
                .build();
            grp.add(&row);
        };

        add_pref_row(&group, "Container Name", &details.name);
        add_pref_row(&group, "Container ID", &details.id);
        add_pref_row(&group, "Image", &details.image);
        add_pref_row(
            &group,
            "State",
            &format!("{} ({})", details.state, details.status),
        );
        add_pref_row(&group, "Created", &details.created);
        add_pref_row(
            &group,
            "Command",
            if details.command.is_empty() {
                "(default)"
            } else {
                &details.command
            },
        );
        add_pref_row(
            &group,
            "Hostname",
            if details.hostname.is_empty() {
                "(none)"
            } else {
                &details.hostname
            },
        );
        add_pref_row(
            &group,
            "Restart Policy",
            if details.restart_policy.is_empty() {
                "no"
            } else {
                &details.restart_policy
            },
        );
        if !details.workdir.is_empty() {
            add_pref_row(&group, "Working Dir", &details.workdir);
        }
        if !details.user.is_empty() {
            add_pref_row(&group, "User", &details.user);
        }

        overview_box.append(&group);

        let scrolled_overview = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&overview_box)
            .build();
        view_stack.add_titled(&scrolled_overview, Some("overview"), "Overview");

        // 2. Ports Tab
        let ports_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();
        let ports_group = adw::PreferencesGroup::builder()
            .title("Port Mappings")
            .description("Published host-to-container ports")
            .build();
        if details.port_mappings.is_empty() {
            let row = adw::ActionRow::builder()
                .title("No published ports")
                .subtitle("Container does not publish any ports to the host")
                .build();
            ports_group.add(&row);
        } else {
            for p in &details.port_mappings {
                let host_display = if p.host_ip.is_empty() {
                    format!("0.0.0.0:{}", p.host_port)
                } else {
                    format!("{}:{}", p.host_ip, p.host_port)
                };
                let row = adw::ActionRow::builder()
                    .title(format!(
                        "{} -> {}/{}",
                        host_display,
                        p.container_port,
                        p.protocol.as_str()
                    ))
                    .subtitle(format!("Protocol: {}", p.protocol.as_str().to_uppercase()))
                    .build();
                ports_group.add(&row);
            }
        }
        ports_box.append(&ports_group);
        let scrolled_ports = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&ports_box)
            .build();
        view_stack.add_titled(&scrolled_ports, Some("ports"), "Ports");

        // 3. Environment Tab
        let env_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();
        let env_group = adw::PreferencesGroup::builder()
            .title("Environment Variables")
            .build();
        if details.env_pairs.is_empty() {
            let row = adw::ActionRow::builder()
                .title("No environment variables configured")
                .build();
            env_group.add(&row);
        } else {
            for e in &details.env_pairs {
                let row = adw::ActionRow::builder()
                    .title(&e.key)
                    .subtitle(&e.value)
                    .build();
                env_group.add(&row);
            }
        }
        env_box.append(&env_group);
        let scrolled_env = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&env_box)
            .build();
        view_stack.add_titled(&scrolled_env, Some("env"), "Environment");

        // 4. Mounts / Volumes Tab
        let mounts_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();
        let mounts_group = adw::PreferencesGroup::builder()
            .title("Volumes & Binds")
            .build();
        if details.volume_mounts.is_empty() {
            let row = adw::ActionRow::builder()
                .title("No mounts configured")
                .build();
            mounts_group.add(&row);
        } else {
            for m in &details.volume_mounts {
                let mode = if m.read_only {
                    "Read-Only"
                } else {
                    "Read-Write"
                };
                let row = adw::ActionRow::builder()
                    .title(format!("{} -> {}", m.source, m.target))
                    .subtitle(format!("Mode: {mode} ({})", m.kind.as_str()))
                    .build();
                mounts_group.add(&row);
            }
        }
        mounts_box.append(&mounts_group);
        let scrolled_mounts = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&mounts_box)
            .build();
        view_stack.add_titled(&scrolled_mounts, Some("mounts"), "Mounts");

        // 5. Networks Tab
        let net_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(16)
            .margin_end(16)
            .build();
        let net_group = adw::PreferencesGroup::builder().title("Networks").build();
        if details.networks.is_empty() {
            let row = adw::ActionRow::builder()
                .title("Network Mode")
                .subtitle(if details.network.is_empty() {
                    "bridge (default)"
                } else {
                    &details.network
                })
                .build();
            net_group.add(&row);
        } else {
            for n in &details.networks {
                let row = adw::ActionRow::builder()
                    .title(n)
                    .subtitle(format!(
                        "Mode: {}",
                        if details.network.is_empty() {
                            "bridge"
                        } else {
                            &details.network
                        }
                    ))
                    .build();
                net_group.add(&row);
            }
        }
        net_box.append(&net_group);
        let scrolled_net = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&net_box)
            .build();
        view_stack.add_titled(&scrolled_net, Some("networks"), "Networks");

        let switcher = adw::ViewSwitcher::builder()
            .stack(&view_stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let switcher_bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .halign(gtk::Align::Center)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        switcher_bar.append(&switcher);

        let content_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        content_box.append(&switcher_bar);
        content_box.append(&view_stack);
        view_stack.set_vexpand(true);

        toolbar.set_content(Some(&content_box));

        let dialog = adw::Dialog::builder()
            .title(format!("Inspect · {}", details.name))
            .content_width(740)
            .content_height(540)
            .child(&toolbar)
            .build();

        let st_clone = state.clone();
        let details_for_dup = details.clone();
        let dialog_clone = dialog.clone();
        duplicate_btn.connect_clicked(move |_| {
            dialog_clone.close();
            let cfg = crate::docker::models::RunConfig::from_details(&details_for_dup);
            let title = format!("Recreate / Duplicate · {}", details_for_dup.name);
            super::run_container_dialog::RunContainerDialog::present_with(
                &st_clone,
                Some(cfg),
                &title,
            );
        });

        dialog.present(None::<&gtk::Widget>);
    }
}
