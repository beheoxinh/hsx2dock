//! Docker Engine install / uninstall helpers (Rancher-style self-management).
//!
//! The app never depends on Docker Desktop: it manages the `docker.service`
//! systemd unit (or a user-configured unit) and the `docker` CLI directly.
//! Install flows escalate with `pkexec` so no blocking root shell is needed.

use std::io;
use std::path::Path;
use std::process::Command;

/// Official Docker Engine install reference shown in the in-app guide.
pub const DOCKER_INSTALL_DOCS_URL: &str = "https://docs.docker.com/engine/install/";

/// Whether the `docker` CLI is on PATH and basically functional.
pub fn cli_available() -> bool {
    let status = Command::new("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !status {
        return which("docker").is_some();
    }
    true
}

fn which(bin: &str) -> Option<String> {
    Command::new("which")
        .arg(bin)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Whether a systemd unit file exists for the given unit (system scope).
pub fn unit_exists(unit: &str) -> bool {
    Command::new("systemctl")
        .arg("cat")
        .arg(unit)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Detect the distro package manager for Engine install commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Apt,
    Dnf,
    Pacman,
    Zypper,
    Unknown,
}

pub fn detect_package_manager() -> PackageManager {
    for (bin, mgr) in [
        ("apt-get", PackageManager::Apt),
        ("dnf", PackageManager::Dnf),
        ("pacman", PackageManager::Pacman),
        ("zypper", PackageManager::Zypper),
    ] {
        if which(bin).is_some() {
            return mgr;
        }
    }
    // Fall back to /etc/os-release hints.
    if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
        let lower = text.to_lowercase();
        if lower.contains("arch") || lower.contains("manjaro") {
            return PackageManager::Pacman;
        } else if lower.contains("suse") || lower.contains("opensuse") {
            return PackageManager::Zypper;
        } else if lower.contains("fedora") || lower.contains("rhel") || lower.contains("centos") {
            return PackageManager::Dnf;
        }
    }
    PackageManager::Unknown
}

/// Escalated shell command that installs Docker Engine for this distro.
/// Run via `pkexec sh -c '<cmd>'` from the UI layer.
pub fn install_command_for_distro() -> String {
    match detect_package_manager() {
        PackageManager::Apt => {
            "apt-get update && apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin && systemctl enable --now docker.service".to_string()
        }
        PackageManager::Dnf => {
            "dnf install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin && systemctl enable --now docker.service".to_string()
        }
        PackageManager::Pacman => {
            "pacman -Sy --noconfirm docker && systemctl enable --now docker.service".to_string()
        }
        PackageManager::Zypper => {
            "zypper install -y docker && systemctl enable --now docker.service".to_string()
        }
        PackageManager::Unknown => {
            "echo 'Unsupported distro: install Docker manually (see docs.docker.com/engine/install)' && exit 1".to_string()
        }
    }
}

/// Escalated shell command that removes Docker Engine packages.
pub fn uninstall_command_for_distro() -> String {
    match detect_package_manager() {
        PackageManager::Apt => {
            "systemctl disable --now docker.service docker.socket; apt-get remove -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin".to_string()
        }
        PackageManager::Dnf => {
            "systemctl disable --now docker.service docker.socket; dnf remove -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin".to_string()
        }
        PackageManager::Pacman => {
            "systemctl disable --now docker.service docker.socket; pacman -Rns --noconfirm docker".to_string()
        }
        PackageManager::Zypper => {
            "systemctl disable --now docker.service docker.socket; zypper remove -y docker".to_string()
        }
        PackageManager::Unknown => {
            "echo 'Unsupported distro: uninstall Docker manually' && exit 1".to_string()
        }
    }
}

/// Run the distro install command under `pkexec` (blocking by design —
/// callers must use `spawn_blocking`).
pub fn install_blocking() -> io::Result<bool> {
    let cmd = install_command_for_distro();
    run_pkexec_sh(&cmd)
}

/// Remove Engine packages under `pkexec`. When `clear_data` is set, also
/// prunes all local Docker data (`docker system prune -af --volumes`).
pub fn uninstall_blocking(clear_data: bool) -> io::Result<bool> {
    let mut cmd = uninstall_command_for_distro();
    if clear_data {
        cmd.push_str("; docker system prune -af --volumes");
    }
    run_pkexec_sh(&cmd)
}

fn run_pkexec_sh(cmd: &str) -> io::Result<bool> {
    let status = Command::new("pkexec").arg("sh").arg("-c").arg(cmd).status()?;
    Ok(status.success())
}

/// Whether a path looks like a Docker data-root worth clearing.
pub fn data_root_exists(path: &Path) -> bool {
    path.join("containers").exists() || path.join("image").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_command_mentions_docker_service_or_manual() {
        let cmd = install_command_for_distro();
        assert!(cmd.contains("docker.service") || cmd.contains("manually"));
    }

    #[test]
    fn uninstall_command_mentions_engine_or_manual() {
        let cmd = uninstall_command_for_distro();
        assert!(cmd.contains("docker") || cmd.contains("manually"));
    }

    #[test]
    fn docs_url_points_at_engine_install() {
        assert!(DOCKER_INSTALL_DOCS_URL.contains("docs.docker.com/engine/install"));
    }
}
