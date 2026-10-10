# Docker Manager

Native GTK4 + libadwaita GUI for managing Docker resources — a simpler, faster
alternative to Docker Desktop. Built in Rust on top of the Docker Engine API
via [bollard](https://crates.io/crates/bollard).

![GTK4 native](https://img.shields.io/badge/GTK-4-blue) ![Rust](https://img.shields.io/badge/Rust-2021-orange)

## Features

- **Engine dashboard** — daemon status (Stopped / Starting / Running),
  engine version, API version, OS/arch, CPU count, total memory, container and
  image counts. One-click Start / Stop of the Docker daemon.
- **Containers** — list running + stopped, search/filter, start, stop,
  restart, delete (force), open an interactive shell (`docker exec -it`,
  bash with sh fallback) in your terminal, and view tailed logs with Follow.
- **Monitoring** — per-container CPU % and memory usage/limit bars, refreshed
  every 3 seconds for running containers via the Docker stats API.
- **Images** — list tags/size/age, search, pull new images with live progress
  (streamed from the daemon), delete, prune unused.
- **Volumes** — list name/driver/mountpoint, create (name + driver), delete,
  prune unused.
- **Networks** — list name/driver/subnet/gateway, create (name + driver),
  delete, prune unused.
- **Compose** — pick a compose file, Up/Down (Down confirmed), service list,
  per-service logs.
- **Registries** — tag/push/history, build with live progress, Docker Hub
  search, login/logout via the `docker` CLI (no secrets stored in-app).
- **Polish** — `Ctrl+R` refresh / `Ctrl+,` preferences shortcuts, header
  working spinner, long-error dialog, copy-ID buttons on every row,
  confirm-before-every-prune/delete.
- **On-demand daemon control** — the app detects the active socket
  (`~/.docker/desktop/docker.sock`, then `/var/run/docker.sock`) and can
  start/stop the daemon via systemd user units (`docker-desktop.service` by
  default, configurable). No daemon? Every action degrades to a toast instead
  of a crash.

## Requirements (Nobara / GNOME)

- GTK 4.12+, libadwaita 1.4+
- Docker Engine or Docker Desktop (socket at `/var/run/docker.sock` or
  `~/.docker/desktop/docker.sock`)
- A terminal emulator for `docker exec`: `ptyxis` (preferred) or
  `gnome-terminal`
- `systemctl --user` for daemon Start/Stop (unit configurable)

## Build & run

```sh
cargo build
./target/debug/docker-manager

# release
cargo build --release
./target/release/docker-manager
```

CLI flags: `--help`, `-V` / `--version`.

## Configuration

Config lives at `~/.config/docker-manager/config.toml` (created on demand):

```toml
auto_start_daemon = false
daemon_unit = "docker-desktop.service"
prefer_socket = ""
```

- `daemon_unit` — systemd user unit used by the Start/Stop buttons.
- `prefer_socket` — force a socket path instead of auto-detection.

## Desktop integration

```sh
# install the .desktop entry and icon
sudo install -Dm644 data/com.github.docker_manager.desktop \
  /usr/share/applications/com.github.docker_manager.desktop
sudo install -Dm644 data/icons/com.github.docker_manager.svg \
  /usr/share/icons/hicolor/scalable/apps/com.github.docker_manager.svg
gtk-update-icon-cache -f /usr/share/icons/hicolor 2>/dev/null || true
```

`desktop-file-validate data/com.github.docker_manager.desktop` passes clean.

## Architecture

```
src/
  main.rs               adw::Application entry, CSS, CLI flags
  runtime.rs            shared Tokio runtime (GTK ↔ async bridge)
  config.rs             TOML config load/save
  terminal_launcher.rs  ptyxis/gnome-terminal `docker exec -it`
  docker/
    client.rs           bollard wrapper (27 methods, all resources)
    daemon.rs           socket detection + systemd unit control
    models.rs           ContainerItem/ImageItem/VolumeItem/NetworkItem/stats
  ui/
    app_state.rs        AppEvent bus, with_client, refresh/toggle daemon
    common.rs           format helpers + main_loop_channel marshalling
    window.rs           ViewStack shell, event drain, poll loops, spinner,
                      shortcuts, error dialog
    dialogs/           run/exec/logs/inspect/pull/push/build/tag/login/hub/preferences
```

Full map: [docs/architecture.md](docs/architecture.md) ·
Usage: [docs/user-guide.md](docs/user-guide.md) ·
Config: [docs/configuration.md](docs/configuration.md) ·
Dev: [docs/development.md](docs/development.md)
    system_view.rs      engine dashboard
    containers_view.rs  containers + stats bars
    container_details_dialog.rs   logs dialog
    images_view.rs      images list
    pull_image_dialog.rs          pull with progress
    volumes_view.rs / networks_view.rs / dialogs.rs
```

Threading rule: GTK widgets are `!Send`. All Docker I/O runs on the shared
Tokio runtime; results return through `main_loop_channel` (async_channel +
`glib::MainContext::spawn_local`) so widgets are only touched on the GTK main
loop.

## Tests

```sh
cargo test            # 6 unit tests
cargo check --all-targets
```

Unit tests cover socket-detection logic (`docker::daemon`), byte/relative-time
formatting (`ui::common`), and the terminal launcher. Live Docker operations
require a running daemon and are exercised manually.
