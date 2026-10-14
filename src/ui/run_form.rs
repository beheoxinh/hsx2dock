use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::Orientation;

use crate::docker::models::{
    EnvVar, Healthcheck, MountKind, PortMapping, PortProtocol, ResourceLimits, RestartPolicy,
    RunConfig, VolumeMount,
};

/// Docker-Desktop-style "Run a new container" form.
///
/// Every field is a structured widget rather than a free-text blob: ports,
/// environment variables and mounts are dynamic row lists that can be added
/// and removed individually, while the remaining Engine options are labelled
/// entries, combo boxes and switches.
///
/// Reading the form back out ([`config`](Self::config)) never fails; the
/// collected [`RunConfig`] is validated by `RunConfig::validate` so that
/// there is a single source of truth for what is acceptable.
pub struct RunConfigForm {
    root: gtk::Box,
    image: gtk::Entry,
    name: gtk::Entry,
    hostname: gtk::Entry,
    cmd: gtk::Entry,
    entrypoint: gtk::Entry,
    workdir: gtk::Entry,
    user: gtk::Entry,
    network: gtk::Entry,
    ports: Rc<RefCell<Vec<Rc<PortRow>>>>,
    ports_box: gtk::Box,
    envs: Rc<RefCell<Vec<Rc<EnvRow>>>>,
    env_box: gtk::Box,
    vols: Rc<RefCell<Vec<Rc<VolumeRow>>>>,
    vol_box: gtk::Box,
    labels: gtk::TextView,
    cap_add: gtk::TextView,
    cap_drop: gtk::TextView,
    extra_hosts: gtk::TextView,
    restart: gtk::DropDown,
    retries: gtk::Entry,
    mem: gtk::Entry,
    cpus: gtk::Entry,
    cpuset: gtk::Entry,
    shares: gtk::Entry,
    pids: gtk::Entry,
    hc_enable: gtk::Switch,
    hc_cmd: gtk::Entry,
    hc_interval: gtk::Entry,
    hc_timeout: gtk::Entry,
    hc_retries: gtk::Entry,
    hc_start: gtk::Entry,
    privileged: gtk::Switch,
    tty: gtk::Switch,
    stdin_open: gtk::Switch,
    auto_remove: gtk::Switch,
    ro_rootfs: gtk::Switch,
    detach: gtk::Switch,
    start_now: gtk::Switch,
}

impl RunConfigForm {
    /// Build the form, optionally pre-filled from an existing configuration
    /// (used by "duplicate" / "recreate" flows).
    pub fn new(initial: Option<&RunConfig>) -> Rc<Self> {
        let image = entry("Image, e.g. nginx:latest");
        let name = entry("Container name (optional)");
        let hostname = entry("Hostname (optional)");
        let cmd = entry("Command override, e.g. nginx -g 'daemon off;'");
        let entrypoint = entry("Entrypoint override (optional)");
        let workdir = entry("Working directory inside the container");
        let user = entry("User or uid:gid (optional)");
        let network = entry("Network name or mode (optional)");

        let ports: Rc<RefCell<Vec<Rc<PortRow>>>> = Rc::new(RefCell::new(Vec::new()));
        let ports_box = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        let envs: Rc<RefCell<Vec<Rc<EnvRow>>>> = Rc::new(RefCell::new(Vec::new()));
        let env_box = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        let vols: Rc<RefCell<Vec<Rc<VolumeRow>>>> = Rc::new(RefCell::new(Vec::new()));
        let vol_box = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();

        let port_spec = entry("Paste a spec, e.g. 127.0.0.1:8080:80/udp, then Add");
        let env_spec = entry("Paste KEY=VALUE, then Add");
        let vol_spec = entry("Paste SOURCE:TARGET[:ro], then Add");

        let labels = text_view(3);
        let cap_add = text_view(2);
        let cap_drop = text_view(2);
        let extra_hosts = text_view(2);

        let restart =
            gtk::DropDown::from_strings(&["no", "always", "unless-stopped", "on-failure"]);
        let retries = entry("Only for on-failure, e.g. 5");

        let mem = entry("MiB, empty = unlimited");
        let cpus = entry("Cores, e.g. 1.5");
        let cpuset = entry("e.g. 0-2,4");
        let shares = entry("2-262144");
        let pids = entry("Max processes");

        let hc_enable = gtk::Switch::new();
        let hc_cmd = entry("CMD-SHELL: curl -f http://localhost/ || exit 1");
        let hc_interval = entry("30s");
        let hc_timeout = entry("5s");
        let hc_retries = entry("3");
        let hc_start = entry("0s");

        let privileged = gtk::Switch::new();
        let tty = gtk::Switch::new();
        let stdin_open = gtk::Switch::new();
        let auto_remove = gtk::Switch::new();
        let ro_rootfs = gtk::Switch::new();
        let detach = gtk::Switch::new();
        detach.set_active(true);
        let start_now = gtk::Switch::new();
        start_now.set_active(true);

        // --- ports: Add button (structured row or pasted spec) -------------
        let add_port = crate::ui::design::button("Add port", None);
        {
            let box_for = ports_box.clone();
            let list = ports.clone();
            let spec = port_spec.clone();
            let grid = port_header();
            box_for.append(&grid);
            add_port.connect_clicked(move |_| {
                let typed = spec.text().trim().to_string();
                let row = if typed.is_empty() {
                    PortRow::new(None)
                } else {
                    match PortMapping::parse(&typed) {
                        Ok(m) => {
                            spec.set_text("");
                            PortRow::new(Some(&m))
                        }
                        Err(e) => {
                            // Leave the bad text in place so it can be fixed.
                            spec.add_css_class("error");
                            spec.set_tooltip_text(Some(&e));
                            return;
                        }
                    }
                };
                spec.remove_css_class("error");
                spec.set_tooltip_text(None);
                let mut l = list.borrow_mut();
                box_for.append(&row.root);
                l.push(row);
            });
        }

        let add_env = crate::ui::design::button("Add variable", None);
        {
            let box_for = env_box.clone();
            let list = envs.clone();
            let spec = env_spec.clone();
            let grid = env_header();
            box_for.append(&grid);
            add_env.connect_clicked(move |_| {
                let typed = spec.text().trim().to_string();
                let row = if typed.is_empty() {
                    EnvRow::new(None)
                } else {
                    match EnvVar::parse(&typed) {
                        Ok(v) => {
                            spec.set_text("");
                            EnvRow::new(Some(&v))
                        }
                        Err(e) => {
                            spec.add_css_class("error");
                            spec.set_tooltip_text(Some(&e));
                            return;
                        }
                    }
                };
                spec.remove_css_class("error");
                spec.set_tooltip_text(None);
                let mut l = list.borrow_mut();
                box_for.append(&row.root);
                l.push(row);
            });
        }

        let add_vol = crate::ui::design::button("Add mount", None);
        {
            let box_for = vol_box.clone();
            let list = vols.clone();
            let spec = vol_spec.clone();
            let grid = vol_header();
            box_for.append(&grid);
            add_vol.connect_clicked(move |_| {
                let typed = spec.text().trim().to_string();
                let row = if typed.is_empty() {
                    VolumeRow::new(None)
                } else {
                    match VolumeMount::parse(&typed) {
                        Ok(m) => {
                            spec.set_text("");
                            VolumeRow::new(Some(&m))
                        }
                        Err(e) => {
                            spec.add_css_class("error");
                            spec.set_tooltip_text(Some(&e));
                            return;
                        }
                    }
                };
                spec.remove_css_class("error");
                spec.set_tooltip_text(None);
                let mut l = list.borrow_mut();
                box_for.append(&row.root);
                l.push(row);
            });
        }

        let root = gtk::Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(10)
            .margin_top(crate::ui::design::SPACE_SM)
            .margin_bottom(crate::ui::design::SPACE_SM)
            .margin_start(crate::ui::design::SPACE_SM)
            .margin_end(crate::ui::design::SPACE_SM)
            .build();

        root.append(&section("Basics"));
        root.append(&labelled_grid(&[
            ("Image", image.clone().upcast()),
            ("Name", name.clone().upcast()),
            ("Hostname", hostname.clone().upcast()),
            ("Command", cmd.clone().upcast()),
            ("Entrypoint", entrypoint.clone().upcast()),
            ("Working dir", workdir.clone().upcast()),
            ("User", user.clone().upcast()),
            ("Network", network.clone().upcast()),
        ]));

        root.append(&section("Ports"));
        root.append(&ports_box);
        root.append(&horizon(&[
            port_spec.clone().upcast(),
            add_port.clone().upcast(),
        ]));

        root.append(&section("Environment"));
        root.append(&env_box);
        root.append(&horizon(&[
            env_spec.clone().upcast(),
            add_env.clone().upcast(),
        ]));

        root.append(&section("Volumes"));
        root.append(&vol_box);
        root.append(&horizon(&[
            vol_spec.clone().upcast(),
            add_vol.clone().upcast(),
        ]));

        root.append(&section("Restart policy"));
        root.append(&labelled_grid(&[
            ("Policy", restart.clone().upcast()),
            ("Max retries", retries.clone().upcast()),
        ]));

        root.append(&section("Resources"));
        root.append(&labelled_grid(&[
            ("Memory", mem.clone().upcast()),
            ("CPUs", cpus.clone().upcast()),
            ("CPU set", cpuset.clone().upcast()),
            ("CPU shares", shares.clone().upcast()),
            ("PIDs limit", pids.clone().upcast()),
        ]));

        root.append(&section("Advanced"));
        root.append(&labelled_grid(&[
            ("Labels (KEY=VALUE per line)", labels.clone().upcast()),
            ("Capabilities to add", cap_add.clone().upcast()),
            ("Capabilities to drop", cap_drop.clone().upcast()),
            ("Extra hosts (host:ip)", extra_hosts.clone().upcast()),
        ]));
        root.append(&switch_row("Privileged", &privileged));
        root.append(&switch_row("Allocate a TTY", &tty));
        root.append(&switch_row("Keep STDIN open", &stdin_open));
        root.append(&switch_row("Read-only root filesystem", &ro_rootfs));
        root.append(&switch_row("Auto-remove when it exits", &auto_remove));

        root.append(&section("Health check"));
        root.append(&switch_row("Enable health check", &hc_enable));
        root.append(&labelled_grid(&[
            ("Command", hc_cmd.clone().upcast()),
            ("Interval", hc_interval.clone().upcast()),
            ("Timeout", hc_timeout.clone().upcast()),
            ("Retries", hc_retries.clone().upcast()),
            ("Start period", hc_start.clone().upcast()),
        ]));

        root.append(&section("Run"));
        root.append(&switch_row("Run in background (detach)", &detach));
        root.append(&switch_row("Start immediately", &start_now));

        let form = Rc::new(Self {
            root,
            image,
            name,
            hostname,
            cmd,
            entrypoint,
            workdir,
            user,
            network,
            ports,
            ports_box,
            envs,
            env_box,
            vols,
            vol_box,
            labels,
            cap_add,
            cap_drop,
            extra_hosts,
            restart,
            retries,
            mem,
            cpus,
            cpuset,
            shares,
            pids,
            hc_enable,
            hc_cmd,
            hc_interval,
            hc_timeout,
            hc_retries,
            hc_start,
            privileged,
            tty,
            stdin_open,
            auto_remove,
            ro_rootfs,
            detach,
            start_now,
        });

        if let Some(cfg) = initial {
            form.load(cfg);
        }
        form
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The image entry, so the dialog can enable/disable its Run button.
    pub fn image_entry(&self) -> &gtk::Entry {
        &self.image
    }

    pub fn is_detached(&self) -> bool {
        self.detach.is_active()
    }

    pub fn start_immediately(&self) -> bool {
        self.start_now.is_active()
    }

    /// Collect the form into a [`RunConfig`]. Never fails; validation happens
    /// in [`RunConfig::validate`].
    pub fn config(&self) -> RunConfig {
        RunConfig {
            image: self.image.text().trim().to_string(),
            name: self.name.text().trim().to_string(),
            hostname: self.hostname.text().trim().to_string(),
            command: split_words(&self.cmd.text()),
            entrypoint: split_words(&self.entrypoint.text()),
            workdir: self.workdir.text().trim().to_string(),
            user: self.user.text().trim().to_string(),
            network: self.network.text().trim().to_string(),
            ports: self
                .ports
                .borrow()
                .iter()
                .filter(|r| r.alive.get())
                .map(|r| r.to_mapping())
                .collect(),
            env: self
                .envs
                .borrow()
                .iter()
                .filter(|r| r.alive.get())
                .map(|r| r.to_var())
                .collect(),
            volumes: self
                .vols
                .borrow()
                .iter()
                .filter(|r| r.alive.get())
                .map(|r| r.to_mount())
                .collect(),
            labels: parse_pairs(&view_text(&self.labels)),
            cap_add: RunConfig::parse_lines(&view_text(&self.cap_add)),
            cap_drop: RunConfig::parse_lines(&view_text(&self.cap_drop)),
            extra_hosts: RunConfig::parse_lines(&view_text(&self.extra_hosts)),
            restart_policy: restart_from_index(self.restart.selected()),
            restart_max_retries: self.retries.text().trim().to_string(),
            resources: ResourceLimits {
                memory_mb: self.mem.text().trim().to_string(),
                cpus: self.cpus.text().trim().to_string(),
                cpuset_cpus: self.cpuset.text().trim().to_string(),
                cpu_shares: self.shares.text().trim().to_string(),
                pids_limit: self.pids.text().trim().to_string(),
            },
            healthcheck: Healthcheck {
                test: health_test(&self.hc_enable, &self.hc_cmd.text()),
                interval_s: self.hc_interval.text().trim().to_string(),
                timeout_s: self.hc_timeout.text().trim().to_string(),
                retries: self.hc_retries.text().trim().to_string(),
                start_period_s: self.hc_start.text().trim().to_string(),
            },
            privileged: self.privileged.is_active(),
            tty: self.tty.is_active(),
            stdin_open: self.stdin_open.is_active(),
            auto_remove: self.auto_remove.is_active(),
            read_only_rootfs: self.ro_rootfs.is_active(),
            detach: self.detach.is_active(),
        }
    }

    /// Replace the form contents with `cfg`, rebuilding the dynamic rows.
    pub fn load(&self, cfg: &RunConfig) {
        self.image.set_text(&cfg.image);
        self.name.set_text(&cfg.name);
        self.hostname.set_text(&cfg.hostname);
        self.cmd.set_text(&cfg.command.join(" "));
        self.entrypoint.set_text(&cfg.entrypoint.join(" "));
        self.workdir.set_text(&cfg.workdir);
        self.user.set_text(&cfg.user);
        self.network.set_text(&cfg.network);

        self.clear_ports();
        for p in &cfg.ports {
            self.add_port_row(Some(p));
        }
        self.clear_envs();
        for e in &cfg.env {
            self.add_env_row(Some(e));
        }
        self.clear_vols();
        for v in &cfg.volumes {
            self.add_vol_row(Some(v));
        }

        set_view_text(&self.labels, &join_pairs(&cfg.labels));
        set_view_text(&self.cap_add, &cfg.cap_add.join("\n"));
        set_view_text(&self.cap_drop, &cfg.cap_drop.join("\n"));
        set_view_text(&self.extra_hosts, &cfg.extra_hosts.join("\n"));

        self.restart.set_selected(restart_index(cfg.restart_policy));
        self.retries.set_text(&cfg.restart_max_retries);
        self.mem.set_text(&cfg.resources.memory_mb);
        self.cpus.set_text(&cfg.resources.cpus);
        self.cpuset.set_text(&cfg.resources.cpuset_cpus);
        self.shares.set_text(&cfg.resources.cpu_shares);
        self.pids.set_text(&cfg.resources.pids_limit);

        self.hc_enable.set_active(cfg.healthcheck.is_enabled());
        self.hc_cmd
            .set_text(&health_test_command(&cfg.healthcheck.test));
        self.hc_interval.set_text(&cfg.healthcheck.interval_s);
        self.hc_timeout.set_text(&cfg.healthcheck.timeout_s);
        self.hc_retries.set_text(&cfg.healthcheck.retries);
        self.hc_start.set_text(&cfg.healthcheck.start_period_s);

        self.privileged.set_active(cfg.privileged);
        self.tty.set_active(cfg.tty);
        self.stdin_open.set_active(cfg.stdin_open);
        self.auto_remove.set_active(cfg.auto_remove);
        self.ro_rootfs.set_active(cfg.read_only_rootfs);
        self.detach.set_active(cfg.detach);
    }

    fn clear_ports(&self) {
        let mut l = self.ports.borrow_mut();
        for row in l.drain(..) {
            self.ports_box.remove(&row.root);
        }
    }

    fn add_port_row(&self, prefill: Option<&PortMapping>) {
        let row = PortRow::new(prefill);
        self.ports_box.append(&row.root);
        self.ports.borrow_mut().push(row);
    }

    fn clear_envs(&self) {
        let mut l = self.envs.borrow_mut();
        for row in l.drain(..) {
            self.env_box.remove(&row.root);
        }
    }

    fn add_env_row(&self, prefill: Option<&EnvVar>) {
        let row = EnvRow::new(prefill);
        self.env_box.append(&row.root);
        self.envs.borrow_mut().push(row);
    }

    fn clear_vols(&self) {
        let mut l = self.vols.borrow_mut();
        for row in l.drain(..) {
            self.vol_box.remove(&row.root);
        }
    }

    fn add_vol_row(&self, prefill: Option<&VolumeMount>) {
        let row = VolumeRow::new(prefill);
        self.vol_box.append(&row.root);
        self.vols.borrow_mut().push(row);
    }
}

/// One editable published-port row.
///
/// Removing a row only hides it and flips `alive`, so the row stays owned by
/// the form's list and [`RunConfigForm::config`] can skip it. That keeps the
/// remove handler free of any reference back into the list.
struct PortRow {
    root: gtk::Box,
    host_ip: gtk::Entry,
    host_port: gtk::Entry,
    container_port: gtk::Entry,
    protocol: gtk::DropDown,
    alive: Rc<std::cell::Cell<bool>>,
}

impl PortRow {
    fn new(prefill: Option<&PortMapping>) -> Rc<Self> {
        let host_ip = entry("0.0.0.0");
        let host_port = entry("8080");
        let container_port = entry("80");
        let protocol = gtk::DropDown::from_strings(&["TCP", "UDP", "SCTP"]);
        if let Some(p) = prefill {
            host_ip.set_text(&p.host_ip);
            host_port.set_text(&p.host_port);
            container_port.set_text(&p.container_port);
            protocol.set_selected(protocol_index(p.protocol));
        }
        let remove = gtk::Button::from_icon_name("list-remove-symbolic");
        remove.set_tooltip_text(Some("Remove this port"));
        remove.add_css_class("flat");

        let root = horizon(&[
            host_ip.clone().upcast(),
            host_port.clone().upcast(),
            container_port.clone().upcast(),
            protocol.clone().upcast(),
            remove.clone().upcast(),
        ]);

        let alive = Rc::new(std::cell::Cell::new(true));
        let row = Rc::new(Self {
            root,
            host_ip,
            host_port,
            container_port,
            protocol,
            alive: alive.clone(),
        });
        {
            let row = row.clone();
            remove.connect_clicked(move |_| {
                alive.set(false);
                row.root.set_visible(false);
            });
        }
        row
    }

    fn to_mapping(&self) -> PortMapping {
        PortMapping {
            host_ip: self.host_ip.text().trim().to_string(),
            host_port: self.host_port.text().trim().to_string(),
            container_port: self.container_port.text().trim().to_string(),
            protocol: protocol_from_index(self.protocol.selected()),
        }
    }
}

/// One editable environment-variable row.
struct EnvRow {
    root: gtk::Box,
    key: gtk::Entry,
    value: gtk::Entry,
    alive: Rc<std::cell::Cell<bool>>,
}

impl EnvRow {
    fn new(prefill: Option<&EnvVar>) -> Rc<Self> {
        let key = entry("KEY");
        let value = entry("value");
        if let Some(v) = prefill {
            key.set_text(&v.key);
            value.set_text(&v.value);
        }
        let remove = gtk::Button::from_icon_name("list-remove-symbolic");
        remove.set_tooltip_text(Some("Remove this variable"));
        remove.add_css_class("flat");

        let root = horizon(&[
            key.clone().upcast(),
            value.clone().upcast(),
            remove.clone().upcast(),
        ]);
        let alive = Rc::new(std::cell::Cell::new(true));
        let row = Rc::new(Self {
            root,
            key,
            value,
            alive: alive.clone(),
        });
        {
            let row = row.clone();
            remove.connect_clicked(move |_| {
                alive.set(false);
                row.root.set_visible(false);
            });
        }
        row
    }

    fn to_var(&self) -> EnvVar {
        EnvVar {
            key: self.key.text().trim().to_string(),
            value: self.value.text().to_string(),
        }
    }
}

/// One editable mount row: bind or named volume.
struct VolumeRow {
    root: gtk::Box,
    kind: gtk::DropDown,
    source: gtk::Entry,
    target: gtk::Entry,
    read_only: gtk::CheckButton,
    alive: Rc<std::cell::Cell<bool>>,
}

impl VolumeRow {
    fn new(prefill: Option<&VolumeMount>) -> Rc<Self> {
        let kind =
            gtk::DropDown::from_strings(&[MountKind::Bind.label(), MountKind::Volume.label()]);
        let source = entry("/host/path or volume name");
        let target = entry("/path/in/container");
        let read_only = gtk::CheckButton::with_label("ro");
        if let Some(m) = prefill {
            source.set_text(&m.source);
            target.set_text(&m.target);
            read_only.set_active(m.read_only);
            kind.set_selected(match m.kind {
                MountKind::Bind => 0,
                MountKind::Volume => 1,
            });
        }
        let remove = gtk::Button::from_icon_name("list-remove-symbolic");
        remove.set_tooltip_text(Some("Remove this mount"));
        remove.add_css_class("flat");

        let root = horizon(&[
            kind.clone().upcast(),
            source.clone().upcast(),
            target.clone().upcast(),
            read_only.clone().upcast(),
            remove.clone().upcast(),
        ]);
        let alive = Rc::new(std::cell::Cell::new(true));
        let row = Rc::new(Self {
            root,
            kind,
            source,
            target,
            read_only,
            alive: alive.clone(),
        });
        {
            let row = row.clone();
            remove.connect_clicked(move |_| {
                alive.set(false);
                row.root.set_visible(false);
            });
        }
        row
    }

    fn to_mount(&self) -> VolumeMount {
        VolumeMount {
            kind: if self.kind.selected() == 1 {
                MountKind::Volume
            } else {
                MountKind::Bind
            },
            source: self.source.text().trim().to_string(),
            target: self.target.text().trim().to_string(),
            read_only: self.read_only.is_active(),
        }
    }
}

/// Index of a protocol in the form's combo box.
pub fn protocol_index(p: PortProtocol) -> u32 {
    match p {
        PortProtocol::Tcp => 0,
        PortProtocol::Udp => 1,
        PortProtocol::Sctp => 2,
    }
}

pub fn protocol_from_index(i: u32) -> PortProtocol {
    match i {
        1 => PortProtocol::Udp,
        2 => PortProtocol::Sctp,
        _ => PortProtocol::Tcp,
    }
}

/// Index of a restart policy in the form's combo box.
pub fn restart_index(p: RestartPolicy) -> u32 {
    match p {
        RestartPolicy::No => 0,
        RestartPolicy::Always => 1,
        RestartPolicy::UnlessStopped => 2,
        RestartPolicy::OnFailure => 3,
    }
}

pub fn restart_from_index(i: u32) -> RestartPolicy {
    match i {
        1 => RestartPolicy::Always,
        2 => RestartPolicy::UnlessStopped,
        3 => RestartPolicy::OnFailure,
        _ => RestartPolicy::No,
    }
}

/// Build the `Healthcheck.test` argv from the switch + the typed command.
///
/// A command starting with `CMD`/`CMD-SHELL`/`NONE` is taken verbatim;
/// anything else is wrapped as `CMD-SHELL` so short shell one-liners work.
pub fn health_test(enabled: &gtk::Switch, text: &str) -> Vec<String> {
    if !enabled.is_active() {
        return Vec::new();
    }
    let raw = text.trim();
    if raw.is_empty() {
        return vec!["NONE".to_string()];
    }
    let upper = raw.to_ascii_uppercase();
    if upper.starts_with("CMD-SHELL")
        || upper.starts_with("CMD ")
        || upper == "NONE"
        || upper == "CMD"
    {
        split_words(raw)
    } else {
        let mut out = vec!["CMD-SHELL".to_string()];
        out.push(raw.to_string());
        out
    }
}

/// Text to show in the health-check command entry for a given `test` argv.
pub fn health_test_command(test: &[String]) -> String {
    if test.is_empty() || test == ["NONE"] {
        return String::new();
    }
    test.join(" ")
}

/// Parse `KEY=VALUE` lines into pairs, skipping blank lines.
pub fn parse_pairs(text: &str) -> Vec<EnvVar> {
    RunConfig::parse_lines(text)
        .iter()
        .filter_map(|l| EnvVar::parse(l).ok())
        .collect()
}

fn join_pairs(vars: &[EnvVar]) -> String {
    vars.iter()
        .map(|v| v.to_line())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Split a command line on whitespace.
pub fn split_words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

fn entry(placeholder: &str) -> gtk::Entry {
    gtk::Entry::builder()
        .placeholder_text(placeholder)
        .hexpand(true)
        .build()
}

fn text_view(lines: i32) -> gtk::TextView {
    let view = gtk::TextView::builder()
        .monospace(true)
        .wrap_mode(gtk::WrapMode::Word)
        .build();
    view.set_size_request(-1, 22 * lines);
    view
}

fn view_text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn set_view_text(view: &gtk::TextView, text: &str) {
    view.buffer().set_text(text);
}

fn horizon(children: &[gtk::Widget]) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(crate::ui::design::SPACE_XS)
        .build();
    for c in children {
        b.append(c);
    }
    b
}

fn section(title: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(title)
        .halign(gtk::Align::Start)
        .css_classes(["heading"])
        .margin_top(crate::ui::design::SPACE_XS)
        .build()
}

fn switch_row(label: &str, switch: &gtk::Switch) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build();
    row.append(
        &gtk::Label::builder()
            .label(label)
            .hexpand(true)
            .halign(gtk::Align::Start)
            .build(),
    );
    row.append(switch);
    row
}

/// Two-column label/widget grid from `(label, widget)` pairs.
fn labelled_grid(rows: &[(&str, gtk::Widget)]) -> gtk::Grid {
    let grid = gtk::Grid::builder()
        .column_spacing(10)
        .row_spacing(crate::ui::design::SPACE_XS)
        .build();
    for (i, (label, widget)) in rows.iter().enumerate() {
        let l = gtk::Label::builder()
            .label(*label)
            .halign(gtk::Align::Start)
            .build();
        grid.attach(&l, 0, i as i32, 1, 1);
        grid.attach(widget, 1, i as i32, 1, 1);
    }
    grid
}

fn port_header() -> gtk::Box {
    header_row(&["Host IP", "Host port", "Container port", "Protocol", ""])
}

fn env_header() -> gtk::Box {
    header_row(&["Key", "Value", ""])
}

fn vol_header() -> gtk::Box {
    header_row(&["Type", "Source", "Target", "ro", ""])
}

fn header_row(cells: &[&str]) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(crate::ui::design::SPACE_XS)
        .build();
    for c in cells {
        b.append(
            &gtk::Label::builder()
                .label(*c)
                .hexpand(true)
                .halign(gtk::Align::Start)
                .css_classes(["dim-label"])
                .build(),
        );
    }
    b
}
