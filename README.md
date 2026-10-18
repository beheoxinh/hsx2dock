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
- **Volumes** — list name/driver/mountpoint with **on-disk size** (symlinks are
  never followed; subtrees that deny permission are skipped and the row is
  flagged *partial* instead of silently under-counting), create (name +
  driver), delete, prune unused.
- **Kubernetes (kind)** — full kind lifecycle with an **isolated in-app
  binary**: pinned release downloaded and checksum-verified into the app's own
  data directory (never your `PATH`), cluster create (custom node image/count)
  / delete with live log streaming, per-cluster kubeconfig path (copied for
  `KUBECONFIG=… kubectl`, always outside `~/.kube`), and one-click
  *Load to cluster* from the Images page (`kind load docker-image`).
- **Networks** — list name/driver/subnet/gateway, create (name + driver),
  delete, prune unused.
- **Compose** — pick a compose file, Up/Down (Down confirmed), service list,
  per-service logs.
- **Registries** — tag/push/history, build with live progress, Docker Hub
  search, login/logout via the `docker` CLI (no secrets stored in-app).
- **System tray & notifications** — live tray icon colored by engine state
  (blue running / gray stopped / amber transitioning) with a dynamic tooltip,
  rich menu (engine status, container/image/cluster counts, Start/Stop engine,
  Open Dashboard), a `Notifications ▸` submenu with recent history and
  per-category toggles, and desktop notifications for build/pull/push
  completion, engine transitions and kind cluster events — every category
  configurable from Settings → Notifications.
- **Polish** — `Ctrl+R` refresh / `Ctrl+,` preferences shortcuts, header
  working spinner, long-error dialog, copy-ID buttons on every row,
  confirm-before-every-prune/delete.
- **Bundled engine** — the app downloads its own Docker Engine (official
  static rootless binaries) into `~/.local/share/docker-manager` on first run
  and runs `dockerd` rootless as a child process on a private socket. Fully
  isolated: the system's Docker install is neither required nor touched.
  No engine yet? Download it from the Engine page (needs network once), then
  every action just works.
- **Docker CLI bridge** — optional switch on the Engine page makes the system
  `docker` command talk to this bundled engine: creates
  `$XDG_RUNTIME_DIR/docker.sock` and a `docker-manager-engine` context, with an
  optional "set as default". Re-applied every 10s while enabled; never touches
  `/var/run/docker.sock` or other contexts.

## Requirements (Nobara / GNOME)

- GTK 4.12+, libadwaita 1.4+
- No system Docker needed — the app bundles its own rootless engine
- A terminal emulator for `docker exec`: `ptyxis` (preferred) or
  `gnome-terminal`

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
auto_start_daemon = true
compose_file = ""
```

- `auto_start_daemon` — start the bundled engine when the app opens.
- `compose_file` — default compose file used by the Compose page.

## Engine layout

The bundled engine lives entirely under the app's own directories — nothing is
written outside them and the host's Docker is never touched.

```
~/.local/share/docker-manager/
  current/bin/            dockerd, docker CLI, rootlesskit, containerd, runc…
  current/cli-plugins/    docker-compose
  current/.complete       written only after checksums verified + extracted
  data/                   containers, images, volumes (survives engine updates)
  log/engine.log          engine stdout/stderr
  engine.pid              pidfile for orphan cleanup
$XDG_RUNTIME_DIR/docker-manager/
  docker.sock             the private API socket the app talks to
```

- First run: the Engine page downloads pinned official binaries, verifies their
  sha256 **before** extracting, then atomically swaps `current` to the new
  version.
- Updates keep `data/` untouched; the `current` symlink swap is atomic, so an
  interrupted update leaves the previous engine in place.
- The engine is a child process: it stops when the app quits. A pidfile + comm
  check cleans up leftovers from a hard-killed app.

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
    daemon.rs           private-socket detection helpers
    engine.rs           bundled engine: paths, download/verify/extract,
                        child-process lifecycle (start/stop), docker_command
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

```
ui/
  app_state.rs        AppEvent bus, with_client, refresh/toggle daemon
  common.rs           format helpers + main_loop_channel marshalling
  window.rs           ViewStack shell, event drain, poll loops, spinner,
                      shortcuts, error dialog
  dialogs/           run/exec/logs/inspect/pull/push/build/tag/login/hub/preferences
  system_view.rs      engine dashboard
  containers_view.rs  containers + stats bars
  container_details_dialog.rs   logs dialog
  images_view.rs      images list
  pull_image_dialog.rs          pull with progress
  volumes_view.rs
  networks_view.rs
  compose_view.rs
  preferences.rs
  build_image_dialog.rs
  run_container_dialog.rs
```
```

Threading rule: GTK widgets are `!Send`. All Docker I/O runs on the shared
Tokio runtime; results return through `main_loop_channel` (async_channel +
`glib::MainContext::spawn_local`) so widgets are only touched on the GTK main
loop.

## Tests

```sh
cargo test            # 31 unit tests
cargo check --all-targets
```

Unit tests cover socket-detection (`docker::daemon`), the engine pipeline
(paths, checksum rejection, tarball extraction, symlink swap, `docker_command`
env) (`docker::engine`), config round-trips, byte/relative-time formatting
(`ui::common`), and the terminal launcher.

The full bundled-engine path (download → verify → extract → rootless start →
`docker run`) was verified manually against a real 29.9.0 static release.
