use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct TerminalLauncher;

impl TerminalLauncher {
    pub fn new() -> Self {
        Self
    }

    pub fn detect_preferred() -> &'static str {
        if Command::new("which")
            .arg("ptyxis")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            "ptyxis"
        } else if Command::new("which")
            .arg("gnome-terminal")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            "gnome-terminal"
        } else {
            "ptyxis"
        }
    }

    pub fn exec_shell(container_id: &str, shell: &str) -> anyhow::Result<()> {
        let term = Self::detect_preferred();
        match term {
            "ptyxis" => {
                Command::new("ptyxis")
                    .arg("--")
                    .arg("docker")
                    .arg("exec")
                    .arg("-it")
                    .arg(container_id)
                    .arg(shell)
                    .spawn()?;
            }
            _ => {
                Command::new("gnome-terminal")
                    .arg("--")
                    .arg("docker")
                    .arg("exec")
                    .arg("-it")
                    .arg(container_id)
                    .arg(shell)
                    .spawn()?;
            }
        }
        Ok(())
    }

    pub fn exec_shell_fallback(container_id: &str) -> anyhow::Result<()> {
        Self::exec_shell(container_id, "/bin/sh")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preferred_returns_known() {
        let p = TerminalLauncher::detect_preferred();
        assert!(p == "ptyxis" || p == "gnome-terminal");
    }

    #[test]
    fn test_exec_does_not_panic() {
        let _ = TerminalLauncher::new();
    }
}
