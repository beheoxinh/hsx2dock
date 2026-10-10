# Architecture

Layer map of the codebase (`src/`).

## Entry & runtime

- `main.rs` — `adw::Application` entry, CSS loading, CLI flags
  (`--socket`, `--config`), single-instance behaviour.
- `runtime.rs` — one shared multi-thread Tokio runtime; GTK stays on the
  main thread. Workers never touch widgets directly.

## Docker layer (`docker/`)

- `client.rs` — bollard wrapper (containers, images, volumes, networks,
  system/df, exec, logs one-shot + follow stream). Log chunks go through
  `LogOutput::into_bytes()`, which already strips multiplex framing.
- `daemon.rs` — socket auto-detection
  (`~/.docker/desktop/docker.sock`, then `/var/run/docker.sock`) plus
  systemd user-unit Start/Stop and health polling.
- `models.rs` — `ContainerItem`, `ContainerDetails`, `RunConfig`
  (with `validate()` + `parse_lines()`), `DfReport`, per-resource items.
- `compose.rs` — `docker compose up/down/ps/logs` orchestration.

## UI layer (`ui/`)

- `app_state.rs` — shared `AppState`: config, daemon manager, optional
  client, and the `AppEvent` bus (`Toast` / `Error` / `Busy(scope)`).
  `with_client()` runs an action on a worker and refreshes the scope;
  `toast()` / `error()` notify synchronously from any thread.
- `common.rs` — `main_loop_channel` (Tokio → GTK marshalling),
  `format_bytes` / `format_relative*` helpers, `copy_to_clipboard`,
  `short_id`.
- `window.rs` — `MainWindow`: header bar (daemon badge, account,
  preferences, refresh, spinner), `ViewStack`, `ShortcutController`
  (`Ctrl+R`, `Ctrl+,`), event-bus drain (`Toast`→toast,
  `Error`→`show_error`, `Busy`→spinner+refresh).
- `*_view.rs` — one view per resource. Rows own their buttons
  (start/stop/…/delete + **copy ID**); destructive actions confirm via
  `adw::AlertDialog` before calling `with_client`.
- `dialogs/` — run / exec / logs / inspect / pull / push / build / tag /
  login / hub-search / prune / preferences dialogs.

## Threading invariant

GTK widgets are `!Send`. Workers capture only `Send` data
(ids, strings, `AppState` handles); UI updates travel through the
`AppEvent` bus and are applied on the glib main loop in
`MainWindow::start_event_bus` → `handle_event`.

## Data flow (example: container start)

1. Row button → `st.with_client(move |client, tx| async move { … })`
2. Worker awaits `client.start_container(&id)`.
3. Worker sends `AppEvent::Toast` / `Busy(Containers)` via `tx`.
4. Main loop drains the bus: toast shown, spinner stops, view refreshes.
