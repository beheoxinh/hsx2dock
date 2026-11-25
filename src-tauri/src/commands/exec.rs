//! Interactive exec sessions and the "Debug" entry point for stopped containers.
//!
//! A session owns a live `POST /exec` attach stream: the frontend pushes
//! keystrokes through `write_exec_stdin`, and the daemon's output is relayed as
//! `exec-output` events (base64-encoded so ANSI/UTF-8 bytes survive the JSON
//! boundary intact, including multi-byte characters split across chunks).

use base64::Engine as _;
use bollard::exec::{ResizeExecOptions, StartExecResults};
use bollard::models::{ContainerStateStatusEnum, ExecConfig};
use futures_util::{FutureExt, StreamExt};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::io::AsyncWriteExt;
use tokio::time::{sleep, Duration, Instant};

use crate::state::{AppState, ExecSession};

/// Shell opened when the caller doesn't ask for a specific command.
const DEFAULT_SHELL: &str = "sh";
/// How long `debug_container` waits for a just-started container to reach the
/// running state before giving up.
const DEBUG_START_TIMEOUT_SECS: u64 = 15;
const DEBUG_POLL_INTERVAL_MS: u64 = 200;
/// Terminal size sanity bounds; absurd values would just confuse the pty.
const MIN_DIM: i32 = 1;
const MAX_DIM: i32 = 500;
/// Upper bound for one coalesced `exec-output` event.
const EXEC_BATCH_BYTES: usize = 64 * 1024;

#[derive(Clone, Serialize)]
pub struct ExecOutputPayload {
    pub session_id: String,
    /// Base64 chunk of raw terminal output; `None` once the stream is done.
    pub data: Option<String>,
    /// True once the exec stream has closed (shell exited / attach dropped).
    pub eof: bool,
}

fn clamp_dim(value: i32) -> u16 {
    value.clamp(MIN_DIM, MAX_DIM) as u16
}

#[cfg(test)]
fn encode_output(log: &bollard::container::LogOutput) -> String {
    base64::engine::general_purpose::STANDARD.encode(log.as_ref())
}

/// Opens an interactive, TTY-backed exec session on a container that is
/// already running.
async fn open_session(
    state: &AppState,
    app: &AppHandle,
    id: &str,
    cmd: Option<Vec<String>>,
    cols: i32,
    rows: i32,
) -> Result<String, String> {
    let client = state.get_client().await?;
    let command: Vec<String> = match cmd {
        Some(c) if !c.is_empty() => c,
        _ => {
            // Honour the user's configured shell; fall back to `sh` if unset.
            let configured = state.config.read().await.terminal_shell.clone();
            let shell = if configured.trim().is_empty() {
                DEFAULT_SHELL.to_string()
            } else {
                configured
            };
            vec![shell]
        }
    };

    let exec = client
        .docker()
        .create_exec(
            id,
            ExecConfig {
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(true),
                cmd: Some(command),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let exec_id = exec.id;

    let started = client
        .docker()
        .start_exec(&exec_id, None)
        .await
        .map_err(|e| e.to_string())?;
    let StartExecResults::Attached { output, input } = started else {
        return Err(
            "The engine started this exec detached; an interactive session is unavailable".into(),
        );
    };

    // Size the pty once it exists (resizing before start is ignored by the
    // daemon) so the first lines don't wrap oddly.
    let _ = client
        .docker()
        .resize_exec(
            &exec_id,
            ResizeExecOptions {
                height: clamp_dim(rows),
                width: clamp_dim(cols),
            },
        )
        .await;

    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

    // Input pump: owns the writer. It ends — and drops the writer, which closes
    // the container-side stdin — once the session is removed from the map and
    // the only sender goes away.
    tokio::spawn(async move {
        let mut input = input;
        while let Some(bytes) = rx.recv().await {
            if input.write_all(&bytes).await.is_err() {
                break;
            }
            let _ = input.flush().await;
        }
    });

    // Output pump: relays every chunk as an event, then signals EOF so the UI
    // can stop waiting for output (the shell exited, or the attach dropped).
    let sid = session_id.clone();
    let emit_app = app.clone();
    let task = tokio::spawn(async move {
        let mut output = output;
        let emit = |data: String| {
            emit_app.emit(
                "exec-output",
                ExecOutputPayload {
                    session_id: sid.clone(),
                    data: Some(data),
                    eof: false,
                },
            )
        };
        while let Some(first) = output.next().await {
            // A burst (`cat` of a big file, `yes`, a build log) arrived as
            // thousands of tiny chunks, one IPC event each, which froze the
            // webview. Coalesce whatever is already buffered into one event;
            // interactive echo still goes out immediately.
            let mut buf: Vec<u8> = Vec::new();
            let mut pending = Some(first);
            while let Some(chunk) = pending.take() {
                match chunk {
                    Ok(log) => buf.extend_from_slice(log.as_ref()),
                    Err(e) => buf.extend_from_slice(format!("exec stream error: {e}\n").as_bytes()),
                }
                if buf.len() >= EXEC_BATCH_BYTES {
                    break;
                }
                pending = output.next().now_or_never().flatten();
            }
            if emit(base64::engine::general_purpose::STANDARD.encode(&buf)).is_err() {
                break;
            }
        }
        let _ = emit_app.emit(
            "exec-output",
            ExecOutputPayload {
                session_id: sid.clone(),
                data: None,
                eof: true,
            },
        );
    });

    // Sweep finished sessions so the map can't grow without bound if the UI
    // never explicitly closed some of them.
    let mut sessions = state.exec_sessions.write().await;
    sessions.retain(|_, s| !s.task.is_finished());
    sessions.insert(
        session_id.clone(),
        ExecSession {
            exec_id,
            stdin_tx: tx,
            task,
        },
    );
    Ok(session_id)
}

/// Starts a stopped (or never-started) container if needed, then opens an
/// interactive shell in it. This is the "Debug" action for containers that are
/// not currently running.
async fn debug_session(
    state: &AppState,
    app: &AppHandle,
    id: &str,
    cmd: Option<Vec<String>>,
    cols: i32,
    rows: i32,
) -> Result<String, String> {
    ensure_running_for_debug(state, id).await?;
    open_session(state, app, id, cmd, cols, rows).await
}

async fn ensure_running_for_debug(state: &AppState, id: &str) -> Result<(), String> {
    let client = state.get_client().await?;
    let inspect = || async {
        client
            .docker()
            .inspect_container(
                id,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .map_err(|e| e.to_string())
    };

    let info = inspect().await?;
    let state_info = info.state;
    if state_info.as_ref().and_then(|s| s.running).unwrap_or(false) {
        return Ok(());
    }
    if state_info.as_ref().and_then(|s| s.paused).unwrap_or(false) {
        return Err("Container is paused — unpause it before debugging".into());
    }

    client
        .start_container(id)
        .await
        .map_err(|e| e.to_string())?;

    // Wait for it to actually come up. A container whose main process exits
    // immediately never becomes debuggable, so report that honestly instead of
    // letting the exec fail with a confusing engine error.
    let deadline = Instant::now() + Duration::from_secs(DEBUG_START_TIMEOUT_SECS);
    loop {
        sleep(Duration::from_millis(DEBUG_POLL_INTERVAL_MS)).await;
        let info = inspect().await?;
        let state_info = info.state;
        if state_info.as_ref().and_then(|s| s.running).unwrap_or(false) {
            return Ok(());
        }
        let status = state_info.as_ref().and_then(|s| s.status);
        if status == Some(ContainerStateStatusEnum::EXITED)
            || status == Some(ContainerStateStatusEnum::DEAD)
        {
            let code = state_info
                .as_ref()
                .and_then(|s| s.exit_code)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".into());
            return Err(format!(
                "The container exited immediately while starting (exit code {code}). \
                 Its main process does not stay running, so a debug shell cannot attach."
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Container did not reach the running state within {DEBUG_START_TIMEOUT_SECS}s"
            ));
        }
    }
}

/// Opens an interactive terminal session on a running container.
#[tauri::command]
pub async fn start_interactive_exec(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    id: String,
    cmd: Option<Vec<String>>,
    cols: i32,
    rows: i32,
) -> Result<String, String> {
    open_session(&state, &app, &id, cmd, cols, rows).await
}

/// Starts the container if it isn't running, then opens an interactive session
/// in it. Used by the "Debug" action on stopped containers.
#[tauri::command]
pub async fn debug_container(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    id: String,
    cmd: Option<Vec<String>>,
    cols: i32,
    rows: i32,
) -> Result<String, String> {
    debug_session(&state, &app, &id, cmd, cols, rows).await
}

/// Pushes keystrokes (or pasted text) into a live session's stdin.
#[tauri::command]
pub async fn write_exec_stdin(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let sessions = state.exec_sessions.read().await;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| "No active terminal session with that id".to_string())?;
    // A session whose output pump ended (shell exited, attach dropped) has no
    // reader left; its input pump may still accept bytes, so say so instead.
    if session.task.is_finished() {
        return Err("The terminal session has closed".to_string());
    }
    session
        .stdin_tx
        .send(data.into_bytes())
        .map_err(|_| "The terminal session has closed".to_string())
}

/// Re-sizes the pty backing a session (called by the terminal's fit observer).
#[tauri::command]
pub async fn resize_exec_session(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    cols: i32,
    rows: i32,
) -> Result<(), String> {
    let exec_id = {
        let sessions = state.exec_sessions.read().await;
        sessions
            .get(&session_id)
            .map(|s| s.exec_id.clone())
            .ok_or_else(|| "No active terminal session with that id".to_string())?
    };
    let client = state.get_client().await?;
    client
        .docker()
        .resize_exec(
            &exec_id,
            ResizeExecOptions {
                height: clamp_dim(rows),
                width: clamp_dim(cols),
            },
        )
        .await
        .map_err(|e| e.to_string())
}

/// Tears a session down: interrupts whatever runs in the foreground, ends the
/// shell, closes stdin and aborts the output pump. Idempotent — a session that
/// already ended is a no-op.
///
/// The engine has no "kill exec" call, and closing stdin alone only ends an
/// idle shell: a `top` or `tail -f` left in the foreground kept running in the
/// container after the terminal tab was closed. Over the pty, Ctrl-C stops
/// the foreground job and Ctrl-D on the then-empty prompt exits the shell.
#[tauri::command]
pub async fn stop_exec_session(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<(), String> {
    let removed = state.exec_sessions.write().await.remove(&session_id);
    if let Some(session) = removed {
        end_session(session);
    }
    Ok(())
}

/// Tears down every live session. The webview calls this on start-up: a
/// reloaded page has forgotten its session ids, so the shells it left running
/// could never be reached or closed from the UI.
#[tauri::command]
pub async fn close_all_exec_sessions(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let drained: Vec<ExecSession> = state
        .exec_sessions
        .write()
        .await
        .drain()
        .map(|(_, session)| session)
        .collect();
    for session in drained {
        end_session(session);
    }
    Ok(())
}

/// Same teardown as [`stop_exec_session`]: Ctrl-C then Ctrl-D over the pty,
/// close stdin, abort the output pump.
fn end_session(session: ExecSession) {
    // The input pump drains queued bytes before it sees the channel close.
    let _ = session.stdin_tx.send(b"\x03\x04".to_vec());
    drop(session.stdin_tx);
    session.task.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::container::LogOutput;

    #[test]
    fn clamp_dim_keeps_terminal_sizes_sane() {
        assert_eq!(clamp_dim(80), 80);
        assert_eq!(clamp_dim(0), 1);
        assert_eq!(clamp_dim(-5), 1);
        assert_eq!(clamp_dim(10_000), 500);
    }

    #[test]
    fn stdout_chunks_encode_losslessly_as_base64() {
        let chunk = LogOutput::StdOut {
            message: bytes::Bytes::from_static(
                b"\x1b[32mchao\x0a ti\xe1\xba\xbfng vi\xe1\xbb\x87t",
            ),
        };
        let encoded = encode_output(&chunk);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("valid base64");
        assert_eq!(
            decoded,
            b"\x1b[32mchao\x0a ti\xe1\xba\xbfng vi\xe1\xbb\x87t"
        );
    }

    #[test]
    fn stderr_and_console_chunks_encode() {
        for log in [
            LogOutput::StdErr {
                message: bytes::Bytes::from_static(b"boom"),
            },
            LogOutput::Console {
                message: bytes::Bytes::from_static(b"tty line"),
            },
        ] {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encode_output(&log))
                .expect("valid base64");
            assert!(!decoded.is_empty());
        }
    }
}
