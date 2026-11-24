//! Running short helper processes without letting a hung one hang the app.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Budget for `--version`-style probes: they answer instantly or not at all.
pub const PROBE: Duration = Duration::from_secs(10);

/// `Command::output` with a deadline. On timeout the child is killed *and*
/// reaped (no zombie left behind) and an error naming `label` is returned.
/// Both pipes are drained on their own threads, so a chatty process cannot
/// block on a full pipe while we wait.
pub fn output_with_timeout(
    mut cmd: Command,
    limit: Duration,
    label: &str,
) -> std::io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out_t = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err_t = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("`{label}` did not finish within {}s and was stopped", limit.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Ok(Output {
        status,
        stdout: out_t.join().unwrap_or_default(),
        stderr: err_t.join().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_output_of_a_quick_process() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo hi; echo err >&2"]);
        let out = output_with_timeout(cmd, Duration::from_secs(5), "sh").unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err");
    }

    #[test]
    fn kills_a_hung_process() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let t = Instant::now();
        let err = output_with_timeout(cmd, Duration::from_millis(200), "sleep").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}
