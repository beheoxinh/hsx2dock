use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::docker::models::DaemonState;
use crate::runtime::spawn;
use crate::ui::app_state::{AppEvent, AppState, RefreshScope};
use crate::ui::compose_view::ComposeView;
use crate::ui::containers_view::ContainersView;
use crate::ui::images_view::ImagesView;
use crate::ui::networks_view::NetworksView;
use crate::ui::system_view::SystemView;
use crate::ui::volumes_view::VolumesView;

/// The main application window: a navigation sidebar, a header bar with the
/// engine status/actions, and a toast overlay for notifications.
pub struct MainWindow {
    window: adw::ApplicationWindow,
    toast_overlay: adw::ToastOverlay,
    state: Rc<AppState>,
    stack: adw::ViewStack,
    system_view: Rc<SystemView>,
    containers_view: Rc<ContainersView>,
    images_view: Rc<ImagesView>,
    volumes_view: Rc<VolumesView>,
    networks_view: Rc<NetworksView>,
    compose_view: Rc<ComposeView>,
    daemon_badge: gtk::Label,
    daemon_btn: gtk::Button,
    account_btn: gtk::Button,
    prefs_btn: gtk::Button,
    refresh_btn: gtk::Button,
    spinner: gtk::Spinner,
}

impl MainWindow {
    pub fn new(app: &adw::Application, state: Rc<AppState>) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Docker Manager")
            .build();
        // Default to ~90% of the usable area of the monitor showing the
        // window so the app opens large without covering top bar / dock.
        if let Some(display) = gtk::gdk::Display::default() {
            if let Some(monitor) = display
                .monitors()
                .item(0)
                .and_downcast::<gtk::gdk::Monitor>()
            {
                let geom = monitor.geometry();
                window.set_default_size(geom.width() * 9 / 10, geom.height() * 9 / 10);
            }
        }

        let system_view = SystemView::new(state.clone());
        let containers_view = ContainersView::new(state.clone());
        let images_view = ImagesView::new(state.clone());
        let volumes_view = VolumesView::new(state.clone());
        let networks_view = NetworksView::new(state.clone());
        let compose_view = ComposeView::new(state.clone());

        let stack = adw::ViewStack::new();

        // The header shows the current section name (the sidebar replaces the
        // old ViewSwitcher).
        let page_title = gtk::Label::builder()
            .label("Engine")
            .css_classes(["title"])
            .build();
        let engine_page = stack.add_titled(system_view.widget(), Some("engine"), "Engine");
        engine_page.set_icon_name(Some("computer-symbolic"));
        let containers_page =
            stack.add_titled(containers_view.widget(), Some("containers"), "Containers");
        containers_page.set_icon_name(Some("applications-system-symbolic"));
        let images_page = stack.add_titled(images_view.widget(), Some("images"), "Images");
        images_page.set_icon_name(Some("image-x-generic-symbolic"));
        let volumes_page = stack.add_titled(volumes_view.widget(), Some("volumes"), "Volumes");
        volumes_page.set_icon_name(Some("drive-harddisk-symbolic"));
        let networks_page = stack.add_titled(networks_view.widget(), Some("networks"), "Networks");
        networks_page.set_icon_name(Some("network-workgroup-symbolic"));
        let compose_page = stack.add_titled(compose_view.widget(), Some("compose"), "Compose");
        compose_page.set_icon_name(Some("package-x-generic-symbolic"));

        // ---- sidebar -------------------------------------------------------
        // One row per section; selecting a row switches the content stack and
        // updates the header title.
        let sidebar_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        let sections: [(&str, &str, &str); 6] = [
            ("engine", "Engine", "computer-symbolic"),
            ("containers", "Containers", "applications-system-symbolic"),
            ("images", "Images", "package-x-generic-symbolic"),
            ("volumes", "Volumes", "drive-harddisk-symbolic"),
            ("networks", "Networks", "network-workgroup-symbolic"),
            ("compose", "Compose", "view-list-symbolic"),
        ];
        for (name, title, icon) in sections {
            let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row_box.append(&gtk::Image::from_icon_name(icon));
            let label = gtk::Label::builder().label(title).xalign(0.0).hexpand(true).build();
            row_box.append(&label);
            let row = gtk::ListBoxRow::builder().child(&row_box).build();
            row.set_widget_name(name);
            sidebar_list.append(&row);
        }

        let sidebar_head = adw::HeaderBar::builder().build();
        let sidebar_title = gtk::Label::builder()
            .label("Docker Manager")
            .css_classes(["title"])
            .build();
        sidebar_head.set_title_widget(Some(&sidebar_title));
        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&sidebar_head);
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&sidebar_list)
            .build();
        sidebar_view.set_content(Some(&sidebar_scroll));
        let sidebar_page = adw::NavigationPage::builder()
            .title("Docker Manager")
            .child(&sidebar_view)
            .build();

        // (Sidebar selection is wired up once the split view exists, below.)

        let daemon_badge = gtk::Label::builder()
            .label("STOPPED")
            .css_classes(["status-pill", "error"])
            .build();
        let daemon_btn = gtk::Button::builder()
            .label("Start")
            .tooltip_text("Start or stop the bundled engine")
            .build();
        daemon_btn.add_css_class("suggested-action");
        let refresh_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh current view")
            .build();
        let prefs_btn = gtk::Button::builder()
            .icon_name("preferences-system-symbolic")
            .tooltip_text("Preferences")
            .build();
        let account_btn = gtk::Button::builder()
            .icon_name("avatar-default-symbolic")
            .tooltip_text("Registry login")
            .build();

        let header = adw::HeaderBar::builder().build();
        header.set_title_widget(Some(&page_title));
        // Engine status reads as one unit: the state badge sits right next to
        // the action that changes it (start / stop / retry).
        let engine_group = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        engine_group.append(&daemon_badge);
        engine_group.append(&daemon_btn);
        let spinner = gtk::Spinner::new();
        spinner.set_tooltip_text(Some("Working…"));
        spinner.stop();
        header.pack_start(&engine_group);
        header.pack_start(&spinner);
        header.pack_end(&refresh_btn);
        header.pack_end(&account_btn);
        header.pack_end(&prefs_btn);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&stack));

        let content_page = adw::NavigationPage::builder()
            .title("Docker Manager")
            .child(&toolbar_view)
            .build();

        let split = adw::NavigationSplitView::builder()
            .sidebar(&sidebar_page)
            .content(&content_page)
            .min_sidebar_width(200.0)
            .max_sidebar_width(280.0)
            .build();

        // Responsive: collapse the sidebar below 700sp so the content uses the
        // whole window; expand again when there is room.
        let collapse = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            700.0,
            adw::LengthUnit::Sp,
        ));
        collapse.add_setter(&split, "collapsed", Some(&true.to_value()));
        let expand = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MinWidth,
            700.0,
            adw::LengthUnit::Sp,
        ));
        expand.add_setter(&split, "collapsed", Some(&false.to_value()));
        window.add_breakpoint(collapse);
        window.add_breakpoint(expand);

        // Selecting a sidebar row switches the stack and reveals the content
        // (so a collapsed sidebar still navigates like a drawer).
        sidebar_list.connect_row_selected({
            let stack = stack.clone();
            let page_title = page_title.clone();
            let split = split.clone();
            move |_, row| {
                if let Some(row) = row {
                    let name = row.widget_name();
                    stack.set_visible_child_name(&name);
                    let title = match name.as_str() {
                        "engine" => "Engine",
                        "containers" => "Containers",
                        "images" => "Images",
                        "volumes" => "Volumes",
                        "networks" => "Networks",
                        "compose" => "Compose",
                        other => other,
                    };
                    page_title.set_label(title);
                    split.set_show_content(true);
                }
            }
        });
        if let Some(first) = sidebar_list.row_at_index(0) {
            sidebar_list.select_row(Some(&first));
        }

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&split));
        window.set_content(Some(&toast_overlay));

        let view = Rc::new(Self {
            window,
            toast_overlay,
            state,
            stack: stack.clone(),
            system_view,
            containers_view,
            images_view,
            volumes_view,
            networks_view,
            compose_view,
            daemon_badge,
            daemon_btn,
            account_btn,
            prefs_btn,
            refresh_btn,
            spinner,
        });

        view.connect_signals();
        view.refresh_all();
        view
    }

    pub fn window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    fn connect_signals(self: &Rc<Self>) {
        // Keyboard shortcuts: Ctrl+R refresh, Ctrl+, preferences.
        let weak = Rc::downgrade(self);
        let refresh_shortcut = gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>r"),
            Some(gtk::CallbackAction::new(move |_, _| {
                if let Some(v) = weak.upgrade() {
                    v.refresh_all();
                }
                glib::Propagation::Stop
            })),
        );
        let weak = Rc::downgrade(self);
        let prefs_shortcut = gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>comma"),
            Some(gtk::CallbackAction::new(move |_, _| {
                if let Some(v) = weak.upgrade() {
                    crate::ui::preferences::PreferencesDialog::present(&v.state);
                }
                glib::Propagation::Stop
            })),
        );
        let controller = gtk::ShortcutController::new();
        controller.add_shortcut(refresh_shortcut);
        controller.add_shortcut(prefs_shortcut);
        self.window.add_controller(controller);
        let weak = Rc::downgrade(self);
        self.daemon_btn.connect_clicked(move |_| {
            tracing::info!("header daemon_btn clicked");
            if let Some(v) = weak.upgrade() {
                v.state.toggle_daemon();
            }
        });

        let weak = Rc::downgrade(self);
        self.prefs_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                crate::ui::preferences::PreferencesDialog::present(&v.state);
            }
        });

        let weak = Rc::downgrade(self);
        self.account_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                crate::ui::login_dialog::LoginDialog::present(&v.state);
            }
        });

        let weak = Rc::downgrade(self);
        self.refresh_btn.connect_clicked(move |_| {
            if let Some(v) = weak.upgrade() {
                v.refresh_all();
            }
        });

        // Refresh the view that becomes visible.
        let weak = Rc::downgrade(self);
        self.stack.connect_visible_child_name_notify(move |stack| {
            if let Some(v) = weak.upgrade() {
                match stack.visible_child_name().as_deref() {
                    Some("engine") => v.state.refresh_daemon(),
                    Some("containers") => v.containers_view.refresh(),
                    Some("images") => v.images_view.refresh(),
                    Some("volumes") => v.volumes_view.refresh(),
                    Some("networks") => v.networks_view.refresh(),
                    Some("compose") => v.compose_view.refresh(),
                    _ => {}
                }
            }
        });

        self.start_event_bus();
        self.state.ensure_daemon_autostart();
        self.start_stats_loop();
        self.start_daemon_poll();
    }

    /// Drain background events on the GTK main loop.
    fn start_event_bus(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let rx = self.state.receiver();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = rx.recv().await {
                let Some(view) = weak.upgrade() else {
                    break;
                };
                view.handle_event(event);
            }
        });
    }

    fn handle_event(self: &Rc<Self>, event: AppEvent) {
        match event {
            AppEvent::DaemonState(state) => {
                self.apply_daemon_state(&state);
                self.system_view.refresh_daemon_state();
                if matches!(state, DaemonState::Running) {
                    self.state.refresh_daemon();
                }
            }
            AppEvent::EngineInfo(info) => {
                let socket = self.state.active_socket().map(|p| p.display().to_string());
                self.system_view.set_engine_info(&info, socket.as_deref());
            }
            AppEvent::DiskUsage(report) => {
                self.system_view.set_df(&report);
            }
            AppEvent::Containers(items) => self.containers_view.set_containers(items),
            AppEvent::ContainerStats(id, stats) => {
                self.containers_view.set_stats(&id, &stats);
            }
            AppEvent::Images(items) => self.images_view.set_images(items),
            AppEvent::Volumes(items) => self.volumes_view.set_volumes(items),
            AppEvent::Networks(items) => self.networks_view.set_networks(items),
            AppEvent::PullDone(Ok(())) => self.toast("Image pull finished"),
            AppEvent::PullDone(Err(e)) => self.toast(&format!("Pull failed: {e}")),
            AppEvent::EngineProgress(label, fraction) => {
                self.system_view.set_engine_progress(&label, fraction);
            }
            AppEvent::Toast(msg) => {
                self.spinner.stop();
                self.toast(&msg);
            }
            AppEvent::Error(msg) => {
                self.spinner.stop();
                // A failed download must not leave the page stuck in "Installing".
                self.state.set_installing(false);
                self.system_view.refresh_daemon_state();
                self.show_error(&msg);
            }
            AppEvent::Busy(scope) => {
                self.spinner.start();
                self.refresh_scope(scope);
            }
            AppEvent::EngineChanged => {
                self.state.invalidate_cli_cache();
                self.state.refresh_daemon();
                self.system_view.refresh_daemon_state();
            }
        }
    }

    fn apply_daemon_state(self: &Rc<Self>, state: &DaemonState) {
        use crate::ui::common::{set_pill, PillTone};
        match state {
            DaemonState::Running => {
                set_pill(&self.daemon_badge, "RUNNING", PillTone::Running);
                self.daemon_btn.set_label("Stop");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("suggested-action");
                self.daemon_btn.add_css_class("destructive-action");
            }
            DaemonState::Starting => {
                set_pill(&self.daemon_badge, "STARTING", PillTone::Warn);
                self.daemon_btn.set_label("Starting…");
                self.daemon_btn.set_sensitive(false);
            }
            DaemonState::Installing => {
                set_pill(&self.daemon_badge, "INSTALLING", PillTone::Warn);
                self.daemon_btn.set_label("Installing…");
                self.daemon_btn.set_sensitive(false);
            }
            DaemonState::Stopping => {
                set_pill(&self.daemon_badge, "STOPPING", PillTone::Warn);
                self.daemon_btn.set_label("Stopping…");
                self.daemon_btn.set_sensitive(false);
            }
            DaemonState::Stopped => {
                set_pill(&self.daemon_badge, "STOPPED", PillTone::Stopped);
                self.daemon_btn.set_label("Start");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("destructive-action");
                self.daemon_btn.add_css_class("suggested-action");
            }
            DaemonState::NotInstalled => {
                set_pill(&self.daemon_badge, "NOT INSTALLED", PillTone::Error);
                self.daemon_btn.set_label("Install");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("destructive-action");
                self.daemon_btn.add_css_class("suggested-action");
            }
            DaemonState::Error(_) => {
                set_pill(&self.daemon_badge, "ERROR", PillTone::Error);
                self.daemon_btn.set_label("Retry");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("destructive-action");
                self.daemon_btn.add_css_class("suggested-action");
            }
        }
    }

    pub fn refresh_all(self: &Rc<Self>) {
        self.state.refresh_daemon();
        self.containers_view.refresh();
        self.images_view.refresh();
        self.volumes_view.refresh();
        self.networks_view.refresh();
        self.compose_view.refresh();
    }

    /// Refresh only the view named by `scope`; daemon completions re-poll
    /// daemon state so the header badge stays honest.
    pub fn refresh_scope(self: &Rc<Self>, scope: RefreshScope) {
        match scope {
            RefreshScope::Daemon => self.state.refresh_daemon(),
            RefreshScope::Containers => self.containers_view.refresh(),
            RefreshScope::Images => self.images_view.refresh(),
            RefreshScope::Volumes => self.volumes_view.refresh(),
            RefreshScope::Networks => self.networks_view.refresh(),
            RefreshScope::Compose => self.compose_view.refresh(),
            RefreshScope::All => self.refresh_all(),
        }
    }

    fn toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        toast.set_timeout(4);
        self.toast_overlay.add_toast(toast);
    }

    /// Short errors become toasts; long ones (>120 chars) get a dialog so the
    /// full message (e.g. a daemon stderr dump) stays readable.
    fn show_error(&self, message: &str) {
        if message.chars().count() <= 120 {
            self.toast(&format!("Error: {message}"));
            return;
        }
        let dlg = adw::AlertDialog::builder()
            .heading("Error")
            .body(message)
            .build();
        dlg.add_response("ok", "OK");
        dlg.set_default_response(Some("ok"));
        dlg.present(Some(&self.window));
    }

    /// Poll stats for every running container every few seconds.
    fn start_stats_loop(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(3, move || {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let ids = view.containers_view.running_ids();
            if ids.is_empty() {
                return glib::ControlFlow::Continue;
            }
            let st = view.state.clone();
            let tx = st.sender();
            if let Some(client) = st.client() {
                spawn(async move {
                    for id in ids {
                        if let Ok(stats) = client.container_stats(&id).await {
                            let _ = tx.send(AppEvent::ContainerStats(id, stats)).await;
                        }
                    }
                });
            }
            glib::ControlFlow::Continue
        });
    }

    /// Poll the daemon socket to keep the connection badge honest.
    fn start_daemon_poll(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let mut last = self.state.daemon_state();
        glib::timeout_add_seconds_local(2, move || {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let now = view.state.daemon_state();
            if now != last {
                last = now.clone();
                view.apply_daemon_state(&now);
                view.system_view.refresh_daemon_state();
                if matches!(now, DaemonState::Running) {
                    view.state.reset_client();
                    view.refresh_all();
                }
            }
            glib::ControlFlow::Continue
        });
    }
}

/// Convenience: keep a minimum window size so the layout never collapses.
pub fn apply_min_size(window: &adw::ApplicationWindow) {
    window.set_size_request(720, 520);
}
