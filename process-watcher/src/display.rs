//! Terminal output for the watcher.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::Path;

use chrono::{DateTime, Local};

use crate::monitor::{ProcessRow, WatchEvent};

/// Formats kilobytes as "850 MB" / "1.2 GB" / "12 KB".
pub fn format_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{} MB", kb / 1024)
    } else {
        format!("{kb} KB")
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

pub struct ScreenInfo<'a> {
    pub total_processes: usize,
    pub top: usize,
    pub interval_ms: u64,
    pub log_path: &'a Path,
}

/// Builds the whole screen as one string, so it is printed in a single write
/// (no flicker).
pub fn render(
    rows: &[ProcessRow],
    recent: &VecDeque<(DateTime<Local>, WatchEvent)>,
    info: &ScreenInfo,
) -> String {
    let mut out = String::new();
    // Clear screen + cursor home.
    out.push_str("\x1B[2J\x1B[H");

    let _ = writeln!(
        out,
        "====== PROCESS WATCHER ======   {}   ({} processes, every {} ms)\n",
        Local::now().format("%H:%M:%S"),
        info.total_processes,
        info.interval_ms
    );
    let _ = writeln!(out, "{:<8}{:<18}{:<3}{:>8}{:>11}", "PID", "NAME", "S", "CPU", "RAM");
    for r in rows.iter().take(info.top) {
        let _ = writeln!(
            out,
            "{:<8}{:<18}{:<3}{:>7.1}%{:>11}",
            r.pid,
            truncate(&r.name, 17),
            r.state,
            r.cpu_percent,
            format_kb(r.rss_kb)
        );
    }

    out.push('\n');
    if recent.is_empty() {
        out.push_str("(no activity yet — watching for new processes...)\n");
    } else {
        for (time, event) in recent {
            let _ = writeln!(out, "{} {event}", time.format("[%H:%M:%S]"));
        }
    }

    let _ = writeln!(out, "\n=============================");
    let _ = writeln!(out, "Logging to {} · Ctrl+C to quit", info.log_path.display());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(format_kb(12), "12 KB");
        assert_eq!(format_kb(870_400), "850 MB");
        assert_eq!(format_kb(1_258_291), "1.2 GB");
    }

    #[test]
    fn renders_rows_and_events() {
        let rows = vec![ProcessRow {
            pid: 1024,
            name: "firefox".into(),
            state: 'S',
            cpu_percent: 12.0,
            rss_kb: 870_400,
        }];
        let mut recent = VecDeque::new();
        recent.push_back((
            Local::now(),
            WatchEvent::Started { pid: 7, ppid: 1, name: "nginx".into(), cmdline: String::new(), exe: None },
        ));
        let screen = render(
            &rows,
            &recent,
            &ScreenInfo { total_processes: 1, top: 10, interval_ms: 2000, log_path: Path::new("x.log") },
        );
        assert!(screen.contains("firefox"));
        assert!(screen.contains("12.0%"));
        assert!(screen.contains("850 MB"));
        assert!(screen.contains("[+] New process detected: nginx (PID 7)"));
    }
}
