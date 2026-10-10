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

/// The main application window: header bar with daemon badge, a view switcher
/// and a toast overlay for notifications.
pub struct MainWindow {
    window: adw::ApplicationWindow,
    toast_overlay: adw::ToastOverlay,
    state: Rc<AppState>,
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
            if let Some(monitor) = display.monitors().item(0).and_downcast::<gtk::gdk::Monitor>()
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

        let switcher = adw::ViewSwitcher::builder().stack(&stack).build();

        let daemon_badge = gtk::Label::builder()
            .label("STOPPED")
            .css_classes(["status-pill", "error"])
            .build();
        let daemon_btn = gtk::Button::builder()
            .label("Start")
            .tooltip_text("Start or stop the Docker daemon")
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

        let header = adw::HeaderBar::builder().title_widget(&switcher).build();
        header.pack_start(&daemon_badge);
        header.pack_start(&daemon_btn);
        header.pack_end(&refresh_btn);
        header.pack_end(&account_btn);
        header.pack_end(&prefs_btn);
        let spinner = gtk::Spinner::new();
        spinner.set_tooltip_text(Some("Working…"));
        spinner.stop();
        header.pack_start(&spinner);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&stack));

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&toolbar_view));
        window.set_content(Some(&toast_overlay));

        let view = Rc::new(Self {
            window,
            toast_overlay,
            state,
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

        view.connect_signals(&stack);
        view.refresh_all();
        view
    }

    pub fn window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    fn connect_signals(self: &Rc<Self>, stack: &adw::ViewStack) {
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
        stack.connect_visible_child_name_notify(move |stack| {
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
            AppEvent::Toast(msg) => {
                self.spinner.stop();
                self.toast(&msg);
            }
            AppEvent::Error(msg) => {
                self.spinner.stop();
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
        match state {
            DaemonState::Running => {
                self.daemon_badge.set_label("RUNNING");
                self.daemon_badge.remove_css_class("error");
                self.daemon_badge.remove_css_class("warning");
                self.daemon_badge.add_css_class("success");
                self.daemon_btn.set_label("Stop");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("suggested-action");
                self.daemon_btn.add_css_class("destructive-action");
            }
            DaemonState::Starting => {
                self.daemon_badge.set_label("STARTING");
                self.daemon_badge.remove_css_class("error");
                self.daemon_badge.remove_css_class("success");
                self.daemon_badge.add_css_class("warning");
                self.daemon_btn.set_label("Starting…");
                self.daemon_btn.set_sensitive(false);
            }
            DaemonState::Stopped => {
                self.daemon_badge.set_label("STOPPED");
                self.daemon_badge.remove_css_class("success");
                self.daemon_badge.remove_css_class("warning");
                self.daemon_badge.add_css_class("error");
                self.daemon_btn.set_label("Start");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("destructive-action");
                self.daemon_btn.add_css_class("suggested-action");
            }
            DaemonState::NotInstalled => {
                self.daemon_badge.set_label("NOT INSTALLED");
                self.daemon_badge.remove_css_class("success");
                self.daemon_badge.add_css_class("error");
                self.daemon_btn.set_label("Install");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("destructive-action");
                self.daemon_btn.add_css_class("suggested-action");
            }
            DaemonState::Error(_) => {
                self.daemon_badge.set_label("ERROR");
                self.daemon_badge.remove_css_class("success");
                self.daemon_badge.add_css_class("error");
                self.daemon_btn.set_label("Retry");
                self.daemon_btn.set_sensitive(true);
                self.daemon_btn.remove_css_class("destructive-action");
                self.daemon_btn.add_css_class("suggested-action");
            }
        }
        self.system_view.refresh_daemon_state();
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
