# Configuration

Hsx2Dock reads a TOML file (override with `--config <path>`):

```toml
# ~/.config/hsx2dock/config.toml
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
- `mcp_enabled` — serve the MCP server over loopback HTTP (off by default; the
  `hsx2dock-mcp` stdio binary needs no setting).
- `mcp_port` — port for that listener, default `45123`.

The MCP bearer token is **not** in this file: it lives in
`~/.config/hsx2dock/mcp.json` with `0600` permissions, because `config.toml` is
routinely copied between machines and pasted into bug reports. Rotate it from
the MCP Server page, which invalidates every agent already configured.

## Preferences dialog

`Ctrl+,` (or the header button) edits the same keys live:
socket override, daemon unit, terminal choice, refresh interval.
Changes apply without restart; secrets are never stored —
`docker login` pipes the password via `--password-stdin` and clears
the entry widget afterwards.

## CLI flags

```sh
hsx2dock --socket /var/run/docker.sock --config ~/.config/hsx2dock/config.toml
```
