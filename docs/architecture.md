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
- `design.rs` — the single source of UI polish: spacing scale
  (`SPACE_XS`/`SM`/`MD`/`LG`), canonical widget constructors
  (`icon_button`, `icon_button_suggested`, `icon_button_destructive`,
  `button`, `button_suggested`, `button_destructive`, `button_flat`),
  status badges (`status_pill` / `retone_pill`, tones `ok`/`warn`/`err`/
  `idle`) and layout helpers (`card`, `page_body`, `hrow`, `vstack`,
  `page_scroller`, `caption`, `mono`). Every view builds its chrome from
  here rather than hand-rolling buttons and margins.
- `anim.rs` — `fade_in`, `spring_to`, `spring`, `revealer`. All of them
  respect the `gtk-enable-animations` setting and degrade to an instant
  transition when the user has motion disabled.
- `system/` — the Engine page, split out of the old single-file
  `system_view.rs`: `mod.rs` (`SystemView` + widget tree), `progress.rs`
  (engine download/start progress), `engine_info.rs` (version/API/OS/
  socket rows), `disk.rs` (disk-usage tiles).
- `run_form.rs` — the Docker-Desktop-style "Run a new container" form:
  port / env / mount / resource-limit editors that build a `RunConfig`.
- `window.rs` — `MainWindow`: header bar (daemon badge, account,
  preferences, refresh, spinner), `ViewStack`, `ShortcutController`
  (`Ctrl+R`, `Ctrl+,`), event-bus drain (`Toast`→toast,
  `Error`→`show_error`, `Busy`→spinner+refresh).
- `*_view.rs` — one view per resource. Rows keep the frequent actions
  inline (start/stop, shell, logs) and push the rest into a
  `view-more-symbolic` overflow menu backed by a `gio::Menu` +
  `gio::SimpleActionGroup` inserted as the `row` action group, so the row
  no longer crams every action into one strip. Destructive actions confirm
  via `adw::AlertDialog` before calling `with_client`.
- `*_dialog.rs` — one flat module per dialog (run form, exec, logs,
  inspect, pull, push, build, container details, login, hub search),
  alongside `dialogs.rs` for the create-volume / create-network / prune
  ones.

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
