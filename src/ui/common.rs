use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_channel::Sender;

/// Bounded bridge from a Tokio worker thread to the GTK main loop.
///
/// GTK widgets are `!Send`, so background futures can never touch them. Create
/// the bridge on the main thread (`f` runs there) and hand the returned `Sender`
/// to the worker: every value sent is delivered to `f` on the main loop.
pub fn main_loop_channel<T, F>(f: F) -> Sender<T>
where
    T: Send + 'static,
    F: Fn(T) + 'static,
{
    let (tx, rx) = async_channel::unbounded::<T>();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(value) = rx.recv().await {
            f(value);
        }
    });
    tx
}

/// Human readable byte size (SI-ish, binary units).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Format a `SystemTime` as a relative "x minutes ago" string.
pub fn format_relative(time: SystemTime) -> String {
    let Ok(elapsed) = SystemTime::now().duration_since(time) else {
        return "unknown".to_string();
    };
    format_duration(elapsed)
}

/// Format a unix timestamp (seconds) as a relative string.
pub fn format_relative_unix(secs: i64) -> String {
    if secs <= 0 {
        return "unknown".to_string();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = now.saturating_sub(secs);
    if delta < 0 {
        return "in the future".to_string();
    }
    format_duration(Duration::from_secs(delta as u64))
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..=59 => format!("{secs} seconds ago"),
        60..=3599 => format!("{} minutes ago", secs / 60),
        3600..=86399 => format!("{} hours ago", secs / 3600),
        _ => format!("{} days ago", secs / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_formatting() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.00 KiB");
        assert_eq!(format_bytes(1024 * 1024 * 3 / 2), "1.50 MiB");
    }

    #[test]
    fn relative_unix_zero_is_unknown() {
        assert_eq!(format_relative_unix(0), "unknown");
    }
}
