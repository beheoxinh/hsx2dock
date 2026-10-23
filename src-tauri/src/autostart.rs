//! "Auto startup on boot" for Linux desktops, via a standard XDG autostart
//! entry. Every mainstream session (GNOME, KDE, XFCE, …) runs the files in
//! `$XDG_CONFIG_HOME/autostart` at login, so no privileged install step or
//! systemd unit is needed — the app owns a plain file it can create and
//! delete as the setting changes.

use std::path::{Path, PathBuf};

/// The file the desktop session scans at login. Named after the app id so it
/// sits next to the installed `.desktop` entry without colliding with it.
const ENTRY_FILE: &str = "hsx2dock.desktop";

/// Argument the autostart entry passes. A boot launch can stay in the tray
/// while a manual launch still opens the window, without the app having to
/// guess why it was started.
pub const START_MINIMIZED_ARG: &str = "--start-minimized";

/// Path of the autostart entry (`$XDG_CONFIG_HOME/autostart/hsx2dock.desktop`).
pub fn entry_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("autostart").join(ENTRY_FILE))
}

/// The `.desktop` body for an autostart-enabled install.
///
/// `Exec` uses the absolute binary path: a login session's `PATH` is not
/// guaranteed to include `~/.local/bin`, where the app installs itself.
pub fn entry_contents(exec: &Path, start_minimized: bool) -> String {
    let mut command = quote_exec(exec);
    if start_minimized {
        command.push(' ');
        command.push_str(START_MINIMIZED_ARG);
    }
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.5\n\
         Name=Hsx2Dock\n\
         Comment=Start Hsx2Dock and its Docker engine at login\n\
         Exec={command}\n\
         Icon=com.github.hsx2dock\n\
         Terminal=false\n\
         Categories=Development;\n\
         StartupWMClass=hsx2dock\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// Desktop-entry `Exec=` quoting. The spec reserves spaces and quotes, so a
/// path containing them is wrapped in double quotes with `\` escaped.
fn quote_exec(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if !raw.contains([' ', '\t', '"', '\'', '\\', '$', '`']) {
        return raw.into_owned();
    }
    let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Makes the on-disk entry match the settings. Idempotent, so it is safe to
/// call on every save and on every launch (a restored/cleaned config dir
/// must not silently lose the login entry).
pub fn apply(enabled: bool, start_minimized: bool) -> std::io::Result<()> {
    let Some(path) = entry_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory to place the autostart entry in",
        ));
    };

    if !enabled {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Already absent is the desired state, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let exec = std::env::current_exe()?;
    std::fs::write(&path, entry_contents(&exec, start_minimized))
}

/// `apply` for a loaded config; keeps the two call sites (save + launch) in
/// step about which flags drive the entry.
pub fn sync(config: &crate::config::AppConfig) -> std::io::Result<()> {
    apply(config.auto_start_on_boot, config.start_minimized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_passes_the_minimized_flag_only_when_asked() {
        let exec = Path::new("/home/u/.local/bin/hsx2dock");

        let normal = entry_contents(exec, false);
        assert!(normal.contains("Exec=/home/u/.local/bin/hsx2dock\n"));
        assert!(!normal.contains(START_MINIMIZED_ARG));
        // Required for the entry to be run by the session at all.
        assert!(normal.starts_with("[Desktop Entry]\n"));
        assert!(normal.contains("Type=Application\n"));
        assert!(normal.contains("X-GNOME-Autostart-enabled=true\n"));

        let hidden = entry_contents(exec, true);
        assert!(hidden.contains(&format!(
            "Exec=/home/u/.local/bin/hsx2dock {START_MINIMIZED_ARG}\n"
        )));
    }

    #[test]
    fn exec_path_with_spaces_is_quoted() {
        let exec = Path::new("/home/u/My Apps/hsx2dock");
        assert!(entry_contents(exec, false).contains("Exec=\"/home/u/My Apps/hsx2dock\"\n"));
    }
}
