# Backend Architecture Report — Tauri v2 + Rust (Docker manager)

Root: `/mnt/WannaBeTheGuy/WwW/Gnome/docker-manager`; Rust under `src-tauri/src`.
Purpose: facts for (A) image-build streaming and (B) remote daemon support.

## 1. AppState — `src-tauri/src/state.rs`

Struct (lines 9-13):
```
pub struct AppState {
    pub client: Arc<RwLock<Option<DockerClient>>>,   // tokio::sync::RwLock, Option = may be unhealthy
    pub engine: Arc<EngineManager>,
    pub config: Arc<RwLock<AppConfig>>,
}
```
- No cached image/container data; `AppState` holds only client + engine + config. Background tick data is emitted, not cached here.
- `AppState::new()` (line 16) is async: `AppConfig::load().await.unwrap_or_default()` (17) → `EnginePaths::with_custom_data_root(Some(&config.data_root))` (18) → `Arc::new(EngineManager::new(paths))` (19) → `let socket = engine.socket()` (20) → `DockerClient::connect(Some(&socket))` (21-25, wrapped in Option, warn-and-None on failure) → returns `Self { client: Arc::new(RwLock::new(client)), engine, config: Arc::new(RwLock::new(config)) }` (27-31).
- `get_client()` (line 49): read-guards `client`; if present and `is_healthy().await` (52) returns `Ok(clone)`; else drops read guard, takes write guard, reconnects via `DockerClient::connect(Some(&self.engine.socket()))` (59-61), caches on success. Lazy reconnect, single cached client — **no multi-daemon concept**.
- `daemon_state()` (line ~40): Running if `engine.is_running()` or `engine.socket_live()`.
- Created in `lib.rs`: `let app_state = crate::runtime::runtime().block_on(AppState::new());` (lib.rs:64), then `.manage(app_state)` (lib.rs:68). Note it is built on a dedicated Tokio runtime (`runtime.rs`) before the Tauri builder, and registered as managed state.

## 2. DockerClient — `src-tauri/src/docker/client.rs`

- Struct (13-15): `#[derive(Clone)] pub struct DockerClient { docker: Docker }` — a thin wrapper over `bollard::Docker`. Cloneable (bollard `Docker` is Arc-backed).
- `pub fn connect(socket: Option<&Path>) -> anyhow::Result<Self>` (19): 
  - `Some(p)` → `let host = format!("unix://{}", p.display()); Docker::connect_with_unix(&host, 120, bollard::API_DEFAULT_VERSION)?` (22-23).
  - `None` → `Docker::connect_with_socket_defaults()?` (25).
  - **UNIX-ONLY.** No `connect_with_http`, `connect_with_ssl`, or `connect_with_ssh` anywhere (verified by repo-wide search — zero matches).
- Socket path source: caller passes `engine.socket()` (from EnginePaths.socket, an app-owned `$XDG_RUNTIME_DIR/docker-manager/docker.sock`, see §7). The `None` branch defers to bollard's env/host defaults.
- Timeout hardcoded 120s (`Timeout` arg), no API version negotiation client-side beyond `API_DEFAULT_VERSION`.
- `is_healthy()` (32) → `ping()` (36) with a 5s `tokio::time::timeout`. Health gate on the cached client.

## 3. bollard version & build API

- `src-tauri/Cargo.toml:20`: `bollard = "0.21"`. Resolved to **0.21.1** (vendored at `~/.cargo/registry/src/index.crates.io-*/bollard-0.21.1`). Also present: `bollard-stubs-1.53.1-rc.29.3.1`.
- Confirmed API in 0.21.1:
  - `pub fn build_image(&self, options: impl Into<crate::query_parameters::BuildImageOptions>, credentials: Option<HashMap<String, DockerCredentials>>, tar: Option<BodyType>) -> impl Stream<Item = Result<BuildInfo, Error>>` (bollard-0.21.1/src/image.rs:655-660).
  - Options type: `bollard::query_parameters::BuildImageOptions` (+ `BuildImageOptionsBuilder`) — defined in bollard-stubs `query_parameters.rs:2117` / builder at 1901; re-exported via `bollard::query_parameters` (`bollard/src/lib.rs:413`).
- **`BuildImageResults` does NOT exist in 0.21.1.** The stream item is `bollard::models::BuildInfo` (`bollard-stubs .../models.rs:369`; struct has `id`, `stream`, `error`, `error_detail`, `status`, `progress`, `aux`, gated fields). `models` re-exported at `bollard-0.21.1/src/lib.rs:388`. If porting older bollard snippets, `BuildImageResults` → `BuildInfo`.
- Connect variants available in 0.21.1 `src/docker.rs`: `connect_with_ssl_defaults` (444), `connect_with_ssl` (498), `connect_with_http_defaults` (599), `connect_with_ssh` (1204), `connect_with_mock` (1267), plus unix variants. So remote transports ARE available in this bollard version; only app-side wiring is missing.
- **Existing (unused) build support:** `DockerClient::build_image<F>(context_dir, dockerfile, tag, on_progress: F)` at client.rs:513-548. Builds a tar via `build_context_tarball` (client.rs:1167, called 523), uses `BuildImageOptionsBuilder` (529), `bollard::body_full(bytes::Bytes::from(tarball))` (536), consumes `self.docker.build_image(opts, None, Some(body))` as a stream (537), forwards `info.stream` lines through the `on_progress` closure and bails on `info.error_detail`. **This method has no tauri command wrapper and no caller** (only references are its own definition/impl) — it is dead code ready to be surfaced.

## 4. pull_image streaming — `src-tauri/src/commands/images.rs`

- Command signature (83): `pub async fn pull_image(app: AppHandle, state: State<'_, AppState>, image: String) -> Result<(), String>`.
- Gets client via `state.get_client().await`; builds `bollard::query_parameters::CreateImageOptionsBuilder` (94).
- Payload struct (72-80, `#[derive(Clone, serde::Serialize)]`):
  `PullProgressPayload { image: String, status: String, progress: Option<String>, current: Option<u64>, total: Option<u64>, done: bool, error: Option<String> }`.
- Stream consumption: `let mut stream = client...` via `futures_util::StreamExt` (import at line 1); `while let Some(info) = stream.next().await`. Spawned via `tokio::spawn` so the command returns immediately (returns `Ok(())` before completion — progress is event-driven, not in the return value).
- Emission: `app_handle.emit("pull-progress", payload)` — **event name `"pull-progress"`**, using the captured `AppHandle` `app_handle` (line 122, 134, 139; errors also emitted as final payload with `done:true`/`error`).
- Frontend listener: `src/pages/...`/`src/lib/...` — `listen("pull-progress", ...)` in `src/lib/` (grep: `src/lib/*` references `pull-progress`). Pattern to copy for the build feature.
- Other emits for reference: `engine-install-progress` (system.rs:351), `engine-state-changed` (system.rs:345/374, lib.rs:32/94), `containers-tick` (lib.rs:38), `container-stats-tick` (lib.rs:53).

## 5. compose_logs — NOT streaming

- `src-tauri/src/commands/compose.rs:55-65`: `pub async fn compose_logs(...) -> Result<String, String>` calls `docker::compose::logs(Path::new(&file_path), tail_n).await` and returns the full string.
- `src-tauri/src/docker/compose.rs` `logs()`: uses `tokio::task::spawn_blocking` to shell out to the bundled compose binary (`run_compose(&file, "logs", &["--no-color", &tail_arg])`), returns `Result<String>`. Not a stream, no emit, no event. So **compose log streaming does not follow the pull_image pattern** — it is a one-shot blocking CLI capture. There is no existing example of streaming subprocess output to the UI.

## 6. Remote / DOCKER_HOST / multi-client support

- **None in the backend client path.** No `connect_with_http|ssl|ssh` usage; `DockerClient::connect` is unix-only.
- `DOCKER_HOST` appears only in `cli_bridge.rs` (comments, 15/165) and `engine.rs` (919-944, 1394, 1766, 2484) — always set to **`unix://<our own socket>`** for the child CLI/bridge/engine processes; never read as an inbound remote endpoint. There is no config field for a remote host/port/TLS/ssh identity.
- `AppConfig` (`src-tauri/src/config.rs`, struct at 14-…) has `auto_start_daemon`, `compose_file`, cli-bridge fields, `data_root`, `theme` — **no daemon endpoint field**. Adding remote support therefore requires a new config field + a client factory that branches on it.

## 6b. EngineManager — `src-tauri/src/docker/engine.rs`

- `EnginePaths` (struct 61-74): fields include `data_root` (dockerd `--data-root`), `versions_dir`, `current_dir` (symlink to active `versions/<ver>`), `bin_dir()` (helper, 111) = `current_dir/bin`, `campaign`-style `run_dir`/`current_dir` etc., and **`socket: PathBuf`** (the app engine's unix socket, `.../docker-manager/docker.sock`), `runtime_dir`, `exec_root`, `rootless_state`, `log`. `with_custom_data_root` (88) constructs all from `dirs::home_dir()` / `dirs::runtime_dir()`; `socket` derives from the runtime dir.
- `EngineManager` (134-140): `struct EngineManager { inner: Arc<Mutex<Inner>>, paths: Arc<std::sync::RwLock<EnginePaths>> }` — owns the bundled dockerd process lifecycle + all paths. `socket()` (164) reads `paths.read().unwrap().socket.clone()`. `set_data_root` (160) rewrites paths via `with_custom_data_root`. `socket_live()` (166+) checks the socket is actually serving.
- **For a non-local endpoint:** none of `EnginePaths` fields are relevant to a remote TCP/SSH daemon — `data_root`, `versions_dir`, `current_dir`, `socket` all describe the *locally bundled* engine. A remote connection needs a separate endpoint descriptor (host/port/tls/ssh), and `EngineManager` should be bypassed (no local dockerd started) for remote mode. The one coupling to break: `AppState::new`/`get_client` always call `DockerClient::connect(Some(&engine.socket()))` — that call site is where a remote branch must be introduced.

## 7. Command registration — `src-tauri/src/lib.rs`

- `init_background_monitoring` (14) spawns tick loop; `run()` (63) builds `AppState`, `.manage(app_state)` (68), setup closure (checks auto-start engine, `set_cli_mode`), then `.invoke_handler(tauri::generate_handler![...])` (131-186) with `.run(tauri::generate_context!())` (188).
- Registered commands: **49** total (containers ~11, images 7, volumes 4, networks 3, compose ~6, system ~18). Images group: `list_images, remove_image, tag_image, prune_images, get_image_history, search_hub, pull_image` (147-153). **No `build_image` command exists.**
- To add feature A: add `images::build_image` command fn (wrapping `DockerClient::build_image`) and append to the `generate_handler!` list.

## Key constraints / implications

- Single hard-wired local unix socket everywhere: `DockerClient::connect(Some(&engine.socket()))` is the sole connect path (state.rs:21, 61). Remote support = new endpoint config + a connection factory branching on it; `EngineManager` local-only fields (socket/data_root/versions) must not be consulted for remote hosts.
- bollard 0.21.1 already exposes `connect_with_http(s)` / `connect_with_ssl` / `connect_with_ssh` and a usable `build_image` stream, so both features are wiring, not dependency upgrades.
- Build streaming follow the exact `pull_image` pattern: `tokio::spawn` + `AppHandle::emit("...", payload)`; a `DockerClient::build_image` with an `on_progress` callback already exists but is unreachable from the UI.
- compose is CLI-shelled and non-streaming — do not use it as the streaming template.
