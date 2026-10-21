# User Guide

How to use Hsx2Dock day to day.

## First run

1. The **Engine** page downloads the **bundled rootless Docker Engine**.
   Download it once (needs internet); it is placed under the app's own data
   directory and runs as this app's child process on a private unix socket.
   Your system's Docker installation is neither required nor touched.
2. Launch the app. The header badge shows engine state:
   **Not Installed / Installing / Starting / Running / Stopped / Error**,
   refreshed every 2s with a live engine probe.
3. Press **Start** on the Engine page when it is stopped. Without a running
   engine every action degrades to a toast — nothing crashes.

## Views

| View | What you can do |
|------|-----------------|
| Engine | Bundled engine status, version, CPU/memory, disk usage, Start/Stop, install/remove, prune-everything |
| Containers | Search, start/stop/restart/pause, shell (`docker exec -it`), logs with Follow, inspect, rename, delete, **copy ID** |
| Images | Search, pull with live progress, tag, push, history, delete, prune, **copy ID** |
| Volumes | Create (name + driver), delete, prune, **copy name** |
| Networks | Create (name + driver), delete, prune, **copy ID** |
| Compose | Pick a `compose.yaml` file, Up / Down (confirmed), live list, per-service logs |

Every page follows the same layout: a **navigation sidebar** on the left lists
all sections (Engine, Containers, Images, Volumes, Networks, Compose) — click a
row to switch. The header bar shows the current section name plus the app-wide
engine status and controls (status badge, Start/Stop, refresh, login,
preferences). Each list page has a toolbar with search and its primary action
directly above the content.

The window is **responsive**: below ~700px wide the sidebar collapses and the
content takes the full width (use the back/menu button in the header to reopen
it). The Engine page lays its data out as a grid of metric tiles and side-by-side
detail/action columns, and list pages span the full content width.

## Keyboard shortcuts

- `Ctrl+R` — refresh all views
- `Ctrl+,` — open Preferences

## Docker CLI from a terminal

The engine runs standalone on a **private socket** the system `docker` command
does not look at by default. The **Docker CLI** panel on the Engine page links
the two, so `docker ...` in any terminal talks to this app's engine.

- Turn the **Docker CLI** switch on. The app then, and every 10s while on:
  - creates `$XDG_RUNTIME_DIR/docker.sock` as a symlink to the engine socket;
  - creates a `docker-manager-engine` context pointing at the engine.
  It never touches `/var/run/docker.sock` or any other context, and nothing is
  written when the switch is off.
- With the switch on, use either:
  - `docker --context docker-manager-engine ps`, or
  - enable **Set as default context** in Preferences (Engine group) so plain
    `docker ps` works.
- The status pill shows **Connected** (context active), **Partial** (mapped but
  not the active context) or **Disconnected**.
- Turning the switch off leaves an existing symlink/context in place; remove
  them with `docker context rm docker-manager-engine` and `rm
  $XDG_RUNTIME_DIR/docker.sock` if you want a clean slate.

Since the link only points at the engine socket, a stopped engine makes
`docker` report "no such file or directory" until it is started again.

## Container shell

`Shell` runs `docker exec -it <container> bash` (falls back to `sh`)
in your terminal: `ptyxis` preferred, else `gnome-terminal`.
Pick the terminal in Preferences.

## Registries & login

- Pull/push dialogs accept `registry/repo:tag`.
- **Docker Hub search** finds public images by keyword.
- **Log in** runs the `docker login` CLI and pipes the password via
  `--password-stdin`; the app never stores your secret.
- **Log out** runs `docker logout`.

## Compose

1. Open the Compose view, choose a `compose.yaml` / `docker-compose.yml`.
2. **Up** (`docker compose up -d`), **Down** asks for confirmation first.
3. Service list + logs update while the project runs.

## Errors

- Short errors appear as toasts.
- Long errors (daemon stderr dumps, >120 chars) open an **Error dialog**
  so the full message stays readable.
- The header spinner animates while background work is in flight.
