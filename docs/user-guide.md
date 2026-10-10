# User Guide

How to use Docker Manager day to day.

## First run

1. Make sure the Docker daemon is reachable: either the system socket
   (`/var/run/docker.sock`) or Docker Desktop's user socket
   (`~/.docker/desktop/docker.sock`).
2. Launch the app. The header badge shows daemon state:
   **Stopped / Starting / Running**.
3. If stopped, press **Start** (systemd user unit, configurable in
   Preferences). Without a daemon every action degrades to a toast —
   nothing crashes.

## Views

| View | What you can do |
|------|-----------------|
| Overview | Engine version, CPU/memory, container+image counts, daemon Start/Stop, prune-everything |
| Containers | Search, start/stop/restart/pause, shell (`docker exec -it`), logs with Follow, inspect, rename, delete, **copy ID** |
| Images | Search, pull with live progress, tag, push, history, delete, prune, **copy ID** |
| Volumes | Create (name + driver), delete, prune, **copy name** |
| Networks | Create (name + driver), delete, prune, **copy ID** |
| Compose | Pick a `compose.yaml` file, Up / Down (confirmed), live list, per-service logs |

## Keyboard shortcuts

- `Ctrl+R` — refresh all views
- `Ctrl+,` — open Preferences

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
