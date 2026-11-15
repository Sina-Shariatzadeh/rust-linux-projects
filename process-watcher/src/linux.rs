//! Readers for the per-process files under /proc/[PID]/ plus the CPU totals
//! from /proc/stat. `parse_*` functions are pure, so they are unit-tested on any OS.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// True when the `/proc` filesystem is available (i.e. we are on Linux).
pub fn proc_available() -> bool {
    Path::new("/proc/stat").exists()
}

/// One reading of one process.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessSample {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub state: char,
    /// utime + stime, in clock ticks since the process started.
    pub cpu_ticks: u64,
    /// Start time in clock ticks after boot — lets us tell a reused PID apart.
    pub start_time: u64,
    pub rss_kb: u64,
    /// Full command line, arguments separated by spaces (empty for kernel threads).
    pub cmdline: String,
    /// Path of the executable (None when permission is denied or it is a kernel thread).
    pub exe: Option<String>,
}

// ---------------------------------------------------------------------------
// /proc/[PID]/stat
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct StatFields {
    pub name: String,
    pub state: char,
    pub ppid: u32,
    pub utime: u64,
    pub stime: u64,
    pub start_time: u64,
}

/// Parses `/proc/[PID]/stat`.
///
/// The second field is the command name in parentheses and may itself contain
/// spaces or ')' characters, so we split on the *last* ')'.
pub fn parse_stat(stat: &str) -> Option<StatFields> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_string();

    // rest[0] is field 3 (state); field N (1-based, see `man 5 proc`) is rest[N - 3].
    let rest: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    let field = |n: usize| rest.get(n - 3).and_then(|v| v.parse::<u64>().ok());

    Some(StatFields {
        name,
        state: rest.first()?.chars().next()?,
        ppid: field(4)? as u32,
        utime: field(14)?,
        stime: field(15)?,
        start_time: field(22)?,
    })
}

// ---------------------------------------------------------------------------
// /proc/[PID]/status
// ---------------------------------------------------------------------------

/// Resident memory (VmRSS) in kB. Kernel threads have no VmRSS line -> 0.
pub fn parse_status_rss_kb(status: &str) -> u64 {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// /proc/[PID]/cmdline
// ---------------------------------------------------------------------------

/// cmdline holds the arguments separated by NUL bytes.
pub fn parse_cmdline(raw: &[u8]) -> String {
    raw.split(|&b| b == 0)
        .filter(|arg| !arg.is_empty())
        .map(String::from_utf8_lossy)
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Reading a whole process
// ---------------------------------------------------------------------------

/// PIDs of all running processes (the numeric directories in /proc).
pub fn list_pids() -> io::Result<Vec<u32>> {
    Ok(fs::read_dir("/proc")?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .collect())
}

/// Reads stat, status, cmdline and exe for one PID.
/// Returns None if the process vanished while we were reading it.
pub fn read_process(pid: u32) -> Option<ProcessSample> {
    let base = PathBuf::from(format!("/proc/{pid}"));
    let stat = parse_stat(&fs::read_to_string(base.join("stat")).ok()?)?;

    let rss_kb = fs::read_to_string(base.join("status"))
        .map(|s| parse_status_rss_kb(&s))
        .unwrap_or(0);
    let cmdline = fs::read(base.join("cmdline"))
        .map(|b| parse_cmdline(&b))
        .unwrap_or_default();
    // exe is a symlink; reading it needs the same user (or root).
    let exe = fs::read_link(base.join("exe"))
        .ok()
        .map(|p| p.display().to_string());

    Some(ProcessSample {
        pid,
        ppid: stat.ppid,
        name: stat.name,
        state: stat.state,
        cpu_ticks: stat.utime + stat.stime,
        start_time: stat.start_time,
        rss_kb,
        cmdline,
        exe,
    })
}

// ---------------------------------------------------------------------------
// /proc/stat — system-wide CPU time
// ---------------------------------------------------------------------------

/// (total CPU ticks across all cores, number of cores)
pub fn read_cpu_total() -> io::Result<(u64, usize)> {
    let s = fs::read_to_string("/proc/stat")?;
    parse_cpu_total(&s)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unexpected /proc/stat format"))
}

pub fn parse_cpu_total(stat: &str) -> Option<(u64, usize)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    // user nice system idle iowait irq softirq steal (guest fields are already in user/nice)
    let total = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(|v| v.parse::<u64>().ok())
        .sum();
    let cores = stat
        .lines()
        .filter(|l| {
            l.strip_prefix("cpu")
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        })
        .count()
        .max(1);
    Some((total, cores))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_with_tricky_name() {
        // A name containing spaces and ')' must not shift the fields.
        let stat = "4242 (my (weird) app) S 1 4242 4242 0 -1 4194560 1200 0 0 0 \
                    150 30 0 0 20 0 3 0 98765 123456789 2048 18446744073709551615";
        let f = parse_stat(stat).unwrap();
        assert_eq!(f.name, "my (weird) app");
        assert_eq!(f.state, 'S');
        assert_eq!(f.ppid, 1);
        assert_eq!(f.utime, 150);
        assert_eq!(f.stime, 30);
        assert_eq!(f.start_time, 98765);
    }

    #[test]
    fn rejects_truncated_stat() {
        assert_eq!(parse_stat("12 (bash) S 1"), None);
        assert_eq!(parse_stat("nonsense"), None);
    }

    #[test]
    fn parses_rss() {
        assert_eq!(parse_status_rss_kb("Name:\tcode\nVmRSS:\t  634880 kB\n"), 634880);
        assert_eq!(parse_status_rss_kb("Name:\tkworker/0:1\n"), 0);
    }

    #[test]
    fn parses_cmdline() {
        assert_eq!(parse_cmdline(b"nginx\0-g\0daemon off;\0"), "nginx -g daemon off;");
        assert_eq!(parse_cmdline(b""), "");
    }

    #[test]
    fn parses_cpu_total() {
        let stat = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\ncpu1 50 0 25 400 25 0 0 0 0 0\n";
        assert_eq!(parse_cpu_total(stat), Some((1000, 2)));
    }
}
