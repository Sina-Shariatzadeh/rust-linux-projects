//! Low-level readers for Linux `/proc` files.
//!
//! Each `read_*` function does the file I/O, and each `parse_*` function is a
//! pure function over the file contents, so the parsing can be unit-tested on
//! any OS.

use std::fs;
use std::io;
use std::path::Path;

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("unexpected {what} format"))
}

/// True when the `/proc` filesystem is available (i.e. we are on Linux).
pub fn proc_available() -> bool {
    Path::new("/proc/stat").exists()
}

// ---------------------------------------------------------------------------
// CPU — /proc/stat
// ---------------------------------------------------------------------------

/// Cumulative CPU time counters (in clock ticks) from the `cpu` line of /proc/stat.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

impl CpuTimes {
    /// CPU usage in percent between an earlier sample and this one.
    pub fn usage_percent_since(&self, prev: &CpuTimes) -> f64 {
        let total = self.total.saturating_sub(prev.total);
        let idle = self.idle.saturating_sub(prev.idle).min(total);
        if total == 0 {
            0.0
        } else {
            100.0 * (total - idle) as f64 / total as f64
        }
    }
}

pub fn read_cpu_times() -> io::Result<CpuTimes> {
    let stat = fs::read_to_string("/proc/stat")?;
    parse_cpu_times(&stat).ok_or_else(|| invalid("/proc/stat"))
}

/// Parses the aggregate line:
/// `cpu  user nice system idle iowait irq softirq steal guest guest_nice`
pub fn parse_cpu_times(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|x| x.parse().unwrap_or(0))
        .collect();
    if v.len() < 4 {
        return None;
    }
    // idle + iowait count as "not busy".
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    // guest/guest_nice are already included in user/nice, so only sum the first 8.
    let total = v.iter().take(8).sum();
    Some(CpuTimes { idle, total })
}

pub fn read_cpu_count() -> io::Result<usize> {
    Ok(parse_cpu_count(&fs::read_to_string("/proc/stat")?))
}

/// Counts the per-core `cpuN` lines.
pub fn parse_cpu_count(stat: &str) -> usize {
    stat.lines()
        .filter(|l| {
            l.strip_prefix("cpu")
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        })
        .count()
}

// ---------------------------------------------------------------------------
// Memory — /proc/meminfo
// ---------------------------------------------------------------------------

/// Memory figures in kilobytes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MemInfo {
    pub total_kb: u64,
    pub available_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

impl MemInfo {
    pub fn used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.available_kb)
    }

    pub fn swap_used_kb(&self) -> u64 {
        self.swap_total_kb.saturating_sub(self.swap_free_kb)
    }
}

pub fn read_meminfo() -> io::Result<MemInfo> {
    Ok(parse_meminfo(&fs::read_to_string("/proc/meminfo")?))
}

pub fn parse_meminfo(meminfo: &str) -> MemInfo {
    let mut m = MemInfo::default();
    let (mut free, mut buffers, mut cached) = (0, 0, 0);
    let mut has_available = false;

    for line in meminfo.lines() {
        let mut parts = line.split_whitespace();
        let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        let value: u64 = value.parse().unwrap_or(0);
        match key {
            "MemTotal:" => m.total_kb = value,
            "MemAvailable:" => {
                m.available_kb = value;
                has_available = true;
            }
            "MemFree:" => free = value,
            "Buffers:" => buffers = value,
            "Cached:" => cached = value,
            "SwapTotal:" => m.swap_total_kb = value,
            "SwapFree:" => m.swap_free_kb = value,
            _ => {}
        }
    }

    // Kernels older than 3.14 have no MemAvailable; approximate it.
    if !has_available {
        m.available_kb = free + buffers + cached;
    }
    m
}

// ---------------------------------------------------------------------------
// Uptime — /proc/uptime
// ---------------------------------------------------------------------------

/// System uptime in seconds.
pub fn read_uptime() -> io::Result<f64> {
    let s = fs::read_to_string("/proc/uptime")?;
    parse_uptime(&s).ok_or_else(|| invalid("/proc/uptime"))
}

/// `/proc/uptime` is "<uptime seconds> <idle seconds>".
pub fn parse_uptime(s: &str) -> Option<f64> {
    s.split_whitespace().next()?.parse().ok()
}

// ---------------------------------------------------------------------------
// Kernel — /proc/version
// ---------------------------------------------------------------------------

pub fn read_kernel_version() -> io::Result<String> {
    let s = fs::read_to_string("/proc/version")?;
    parse_kernel_version(&s).ok_or_else(|| invalid("/proc/version"))
}

/// "Linux version 6.8.0-45-generic (buildd@...) ..." -> "Linux 6.8.0-45-generic"
pub fn parse_kernel_version(s: &str) -> Option<String> {
    let mut it = s.split_whitespace();
    let os = it.next()?;
    if it.next()? != "version" {
        return None;
    }
    let release = it.next()?;
    Some(format!("{os} {release}"))
}

// ---------------------------------------------------------------------------
// Processes — /proc/[pid]/status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    /// e.g. "S (sleeping)"
    pub state: String,
    pub rss_kb: u64,
    pub threads: u32,
}

/// Lists every process by scanning the numeric directories in /proc.
pub fn list_processes() -> io::Result<Vec<ProcessInfo>> {
    let mut out = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        // A process can exit between read_dir and this read; just skip it.
        if let Ok(status) = fs::read_to_string(entry.path().join("status")) {
            if let Some(p) = parse_status(pid, &status) {
                out.push(p);
            }
        }
    }
    Ok(out)
}

pub fn parse_status(pid: u32, status: &str) -> Option<ProcessInfo> {
    let mut name = None;
    let mut state = String::new();
    let mut rss_kb = 0;
    let mut threads = 0;

    for line in status.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let first_num = || value.split_whitespace().next().and_then(|v| v.parse().ok());
        match key {
            "Name" => name = Some(value.to_string()),
            "State" => state = value.to_string(),
            // Kernel threads have no VmRSS line, so they stay at 0.
            "VmRSS" => rss_kb = first_num().unwrap_or(0),
            "Threads" => threads = first_num().unwrap_or(0) as u32,
            _ => {}
        }
    }

    Some(ProcessInfo {
        pid,
        name: name?,
        state,
        rss_kb,
        threads,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "\
cpu  4705 356 584 3699 23 0 23 0 0 0
cpu0 1393 280 290 3200 13 0 13 0 0 0
cpu1 3312 76 294 499 10 0 10 0 0 0
intr 114930548 113199788 3 0 5 263 0 4 [... lots more numbers ...]
ctxt 1990473
";

    #[test]
    fn parses_cpu_times() {
        let t = parse_cpu_times(STAT).unwrap();
        assert_eq!(t.idle, 3699 + 23);
        assert_eq!(t.total, 4705 + 356 + 584 + 3699 + 23 + 23);
    }

    #[test]
    fn counts_cores() {
        assert_eq!(parse_cpu_count(STAT), 2);
    }

    #[test]
    fn computes_usage_between_samples() {
        let a = CpuTimes { idle: 100, total: 200 };
        let b = CpuTimes { idle: 150, total: 400 };
        // 200 ticks elapsed, 50 idle -> 75% busy
        assert!((b.usage_percent_since(&a) - 75.0).abs() < 1e-9);
        assert_eq!(a.usage_percent_since(&a), 0.0);
    }

    #[test]
    fn parses_meminfo() {
        let m = parse_meminfo(
            "MemTotal:       16303428 kB\n\
             MemFree:         1234567 kB\n\
             MemAvailable:   12000000 kB\n\
             SwapTotal:       2097148 kB\n\
             SwapFree:        1572864 kB\n",
        );
        assert_eq!(m.total_kb, 16303428);
        assert_eq!(m.used_kb(), 16303428 - 12000000);
        assert_eq!(m.swap_used_kb(), 2097148 - 1572864);
    }

    #[test]
    fn meminfo_falls_back_without_memavailable() {
        let m = parse_meminfo("MemTotal: 1000 kB\nMemFree: 100 kB\nBuffers: 50 kB\nCached: 200 kB\n");
        assert_eq!(m.available_kb, 350);
    }

    #[test]
    fn parses_uptime_and_kernel() {
        assert_eq!(parse_uptime("43200.55 170000.10\n"), Some(43200.55));
        assert_eq!(
            parse_kernel_version("Linux version 6.8.0-45-generic (buildd@lcy02) (gcc 13.2.0) #45-Ubuntu SMP"),
            Some("Linux 6.8.0-45-generic".to_string())
        );
        assert_eq!(parse_kernel_version("garbage"), None);
    }

    #[test]
    fn parses_process_status() {
        let p = parse_status(
            1024,
            "Name:\tfirefox\nState:\tS (sleeping)\nThreads:\t87\nVmRSS:\t  870400 kB\n",
        )
        .unwrap();
        assert_eq!(p.name, "firefox");
        assert_eq!(p.state, "S (sleeping)");
        assert_eq!(p.rss_kb, 870400);
        assert_eq!(p.threads, 87);
    }
}
