# Configuration

Hsx2Dock reads a TOML file (override with `--config <path>`):

```toml
# ~/.config/docker-manager/config.toml
socket = "/var/run/docker.sock"   # optional: force a socket path
daemon_unit = "docker.service"    # systemd user unit for Start/Stop
terminal = "ptyxis"               # ptyxis | gnome-terminal | auto
refresh_interval = 3              # stats polling, seconds
```

## Keys

- `prefer_socket` — force a socket path instead of auto-detection
  (`~/.docker/desktop/docker.sock` first, then `/var/run/docker.sock`).
- `daemon_unit` — systemd user unit used by the Start/Stop buttons
  (default targets the Docker Desktop unit; point it at
  `docker.service` for a system engine).
- `terminal` — terminal used for `docker exec -it`: `ptyxis`,
  `gnome-terminal`, or `auto` (prefer ptyxis, fall back).
- `refresh_interval` — container stats polling period in seconds.

## Preferences dialog

`Ctrl+,` (or the header button) edits the same keys live:
socket override, daemon unit, terminal choice, refresh interval.
Changes apply without restart; secrets are never stored —
`docker login` pipes the password via `--password-stdin` and clears
the entry widget afterwards.

## CLI flags

```sh
hsx2dock --socket /var/run/docker.sock --config ~/.config/docker-manager/config.toml
```
