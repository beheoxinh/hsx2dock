# Hsx2Dock

Modern cross-platform Docker GUI — a Tauri v2 desktop app with a
React/TypeScript frontend and a Rust backend built on the Docker Engine API via
[bollard](https://crates.io/crates/bollard). It ships its own isolated rootless
Docker engine and manages kind (Kubernetes-in-Docker) clusters.

![Tauri 2](https://img.shields.io/badge/Tauri-2-blue) ![Rust](https://img.shields.io/badge/Rust-2021-orange) ![React](https://img.shields.io/badge/React-19-61dafb)

## Features

- **Engine dashboard** — daemon status (Stopped / Starting / Running),
  engine version, API version, OS/arch, CPU count, total memory, container and
  image counts. One-click Start / Stop of the Docker daemon.
- **Containers** — list running + stopped, search/filter, start, stop,
  restart, rename, delete (force), edit resource limits (memory / CPUs /
  cpuset / pids), open an embedded terminal (`docker exec -it`, shell
  auto-detected, debug container fallback for distroless images) or a **Debug**
  shell that starts a stopped container first, and view tailed logs with Follow.
- **Monitoring** — per-container CPU % and memory usage/limit bars, refreshed
  every 3 seconds for running containers via the Docker stats API.
- **Images** — list tags/size/age, search, pull new images with live progress
  (streamed from the daemon), delete, prune unused, **add a tag** to a local
  image, and inspect the **layer history** of any image.
- **Volumes** — list name/driver/mountpoint with **on-disk size** (symlinks are
  never followed; subtrees that deny permission are skipped and the row is
  flagged *partial* instead of silently under-counting), create (name +
  driver), delete, prune unused.
- **Kubernetes (kind)** — full kind lifecycle with an **isolated in-app
  binary**: pinned release downloaded and checksum-verified into the app's own
  data directory (never your `PATH`), cluster create (custom node image/count)
  / delete with live log streaming, per-cluster kubeconfig path (copied for
  `KUBECONFIG=… kubectl`, always outside `~/.kube`), and one-click
  *Load to cluster* from the Images page (`kind load docker-image`). Per
  cluster: CPU/RAM charts, pods, workloads, services, config/secrets, PVCs,
  events, pod terminal/YAML/logs, and metrics-server install. Cluster
  infrastructure (node images and containers, the `kind` network and node
  volumes) is hidden from the generic resource lists and excluded from every
  delete and prune path — it is only managed from the Kubernetes page.
- **Auto-start** — a master switch plus a per-cluster switch decide which
  clusters start with the engine; starts are staggered and node containers are
  pinned to `restart=no` so Docker never resurrects a cluster on its own.
- **Networks** — list name/driver/subnet/gateway, create (name + driver),
  delete, prune unused.
- **Compose** — pick a compose file, Up/Down (Down confirmed), service list,
  per-service logs.
- **Registries** — tag/push/history, build with live progress, Docker Hub
  search, login/logout via the `docker` CLI (no secrets stored in-app).
- **System tray & notifications** — live tray icon colored by engine state,
  dynamic tooltip, rich menu (engine status, container/image/cluster counts,
  Start/Stop engine, Open Dashboard), a `Notifications ▸` submenu with recent
  history, and per-category desktop notifications (build/pull/push, engine
  transitions, kind events), configurable from Settings → Notifications.
- **Bundled engine** — the app downloads its own Docker Engine (official
  static rootless binaries) into `~/.local/share/hsx2dock` on first run
  and runs `dockerd` rootless as a child process on a private socket. Fully
  isolated: the system's Docker install is neither required nor touched.
- **Binary updates** — Settings → Updates checks Docker Engine, Docker CLI,
  Docker Compose and kind against their upstream releases and updates each one
  step by step (stop → download → verify checksum → replace → verify →
  restart) with live progress bars and logs. A release only becomes current
  after it verifies, so an interrupted update always leaves a working binary.
- **Docker CLI bridge** — optional switch on the Engine page makes the system
  `docker` command talk to this bundled engine: creates
  `$XDG_RUNTIME_DIR/docker.sock` and a `hsx2dock-engine` context, with an
  optional "set as default". Re-applied every 10s while enabled; never touches
  `/var/run/docker.sock` or other contexts.

## Requirements (Nobara / GNOME)

- WebKitGTK 4.1 (the Tauri 2 webview runtime)
- No system Docker needed — the app bundles its own rootless engine
- Network access the first time the engine (and the kind binary) is installed

## Build & run

```sh
pnpm install
pnpm tauri dev

# release + system install (binary `hsx2dock`, .desktop entry and icon)
./install.sh
```

`./install.sh` builds the release binary and installs:

- `~/.local/bin/hsx2dock`
- `~/.local/share/applications/com.github.hsx2dock.desktop`
- `~/.local/share/icons/hicolor/scalable/apps/com.github.hsx2dock.svg`

## Configuration

Config lives at `~/.config/hsx2dock/config.toml` (created on demand):

```toml
auto_start_daemon = true
compose_file = ""
```

- `auto_start_daemon` — start the bundled engine when the app opens.
- `compose_file` — default compose file used by the Compose page.
- `data_root` — where containers/images/volumes live (survives engine updates).
- `kind_auto_start` / `kind_cluster_prefs` — cluster auto-start switches.

The internal directories are named `hsx2dock` too. An install created by an
older build keeps working: on first launch the app renames the former
`docker-manager` config/data/runtime trees to `hsx2dock` and leaves the old
paths behind as symlinks, so engine state, kind clusters and the old
`docker-manager-engine` context are migrated rather than lost.

## Engine layout

The bundled engine lives entirely under the app's own directories — nothing is
written outside them and the host's Docker is never touched.

```
~/.local/share/hsx2dock/
  current/bin/            dockerd, docker CLI, rootlesskit, containerd, runc…
  current/cli-plugins/    docker-compose
  current/.complete       written only after checksums verified + extracted
  data/                   containers, images, volumes (survives engine updates)
  log/engine.log          engine stdout/stderr
  engine.pid              pidfile for orphan cleanup
$XDG_RUNTIME_DIR/hsx2dock/
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

`./install.sh` does all of this for you. Manually:

```sh
install -Dm755 src-tauri/target/release/hsx2dock ~/.local/bin/hsx2dock
install -Dm644 data/com.github.hsx2dock.desktop \
  ~/.local/share/applications/com.github.hsx2dock.desktop
install -Dm644 data/icons/com.github.hsx2dock.svg \
  ~/.local/share/icons/hicolor/scalable/apps/com.github.hsx2dock.svg
update-desktop-database ~/.local/share/applications 2>/dev/null || true
```

## Architecture

```
src-tauri/src/            Rust backend (Tauri commands + engine supervision)
  main.rs                 binary entry (`hsx2dock_lib::run()`)
  lib.rs                  Tauri builder, command registration, background monitor
  state.rs                AppState (engine, client, config, exec sessions)
  runtime.rs              shared Tokio runtime / engine-op marshalling
  config.rs               TOML config load/save
  tray.rs                 tray icon, menu, notifications
  commands/               engine, containers, images, volumes, networks,
                          compose, kind/k8s, exec, registries, groups…
  docker/                 bollard client wrapper, bundled-engine management,
                          CLI bridge, kind paths
src/                      React frontend
  pages/                  Dashboard, Containers, Images, Volumes, Networks,
                          Compose, Kubernetes, Settings
  components/             UI primitives, k8s monitor, terminal, settings cards
  lib/api.ts              typed `invoke` wrappers for every Tauri command
  types/index.ts          shared payload types
```

## Tests

```sh
cd src-tauri && cargo test --lib    # unit tests (156)
pnpm build                          # tsc + vite production build
```
