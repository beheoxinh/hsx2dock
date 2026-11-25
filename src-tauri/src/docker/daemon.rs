//! Socket helpers for the bundled engine.
//!
//! The app talks to exactly one daemon — the rootless engine it owns and runs
//! as a child process (`crate::docker::engine`). There is no systemd unit and
//! no fallback to the host's `/var/run/docker.sock`: that fallback is what
//! would leak the app back into the OS engine, so it is deliberately absent.

use std::path::Path;

use crate::docker::models::DaemonState;

/// A socket path is usable when it exists and accepts a unix connection.
/// A stale socket file (e.g. left over from a hard-killed engine) must not
/// count as a live daemon. Shares the engine's non-blocking probe: a blocking
/// connect waits on a full listen backlog, and this is called from async code.
pub fn is_usable_socket(path: &Path) -> bool {
    crate::docker::engine::is_usable_socket(path)
}

/// State derived purely from socket reachability.
pub fn state_from_socket(socket: &Path) -> DaemonState {
    if is_usable_socket(socket) {
        DaemonState::Running
    } else {
        DaemonState::Stopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn missing_socket_is_not_usable() {
        let p = PathBuf::from("/nonexistent/docker.sock");
        assert!(!is_usable_socket(&p));
        assert_eq!(state_from_socket(&p), DaemonState::Stopped);
    }

    #[test]
    fn plain_file_is_not_a_live_daemon() {
        let dir = std::env::temp_dir().join(format!("dm-sock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("docker.sock");
        std::fs::write(&f, b"not a socket").unwrap();
        assert!(!is_usable_socket(&f));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
