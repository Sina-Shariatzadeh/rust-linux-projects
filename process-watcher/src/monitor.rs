//! Tracks processes between ticks: computes CPU %, detects new and exited
//! processes, and raises high-CPU / high-memory alerts.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::io;

use crate::linux::{self, ProcessSample};

/// An alert clears only after usage drops below this fraction of the
/// threshold, so a process hovering around the limit doesn't spam alerts.
const HYSTERESIS: f64 = 0.8;

#[derive(Debug, Clone)]
pub struct Config {
    /// CPU % (100 = one full core) that triggers a high-CPU alert.
    pub cpu_threshold: f64,
    /// Resident memory in kB that triggers a high-memory alert.
    pub mem_threshold_kb: Option<u64>,
    /// Only watch processes whose name or command line contains this (lowercase).
    pub filter: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessRow {
    pub pid: u32,
    pub name: String,
    pub state: char,
    pub cpu_percent: f64,
    pub rss_kb: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WatchEvent {
    Started { pid: u32, ppid: u32, name: String, cmdline: String, exe: Option<String> },
    Exited { pid: u32, name: String },
    HighCpu { pid: u32, name: String, cpu: f64 },
    HighMem { pid: u32, name: String, rss_kb: u64 },
}

impl WatchEvent {
    /// Detailed single-line form for the log file.
    pub fn log_line(&self) -> String {
        match self {
            WatchEvent::Started { pid, ppid, name, cmdline, exe } => format!(
                "[+] START pid={pid} ppid={ppid} name={name} exe={} cmd={}",
                exe.as_deref().unwrap_or("?"),
                if cmdline.is_empty() { "-" } else { cmdline }
            ),
            WatchEvent::Exited { pid, name } => format!("[-] EXIT  pid={pid} name={name}"),
            WatchEvent::HighCpu { pid, name, cpu } => {
                format!("[!] CPU   pid={pid} name={name} cpu={cpu:.1}%")
            }
            WatchEvent::HighMem { pid, name, rss_kb } => {
                format!("[!] MEM   pid={pid} name={name} rss={rss_kb}kB")
            }
        }
    }
}

/// Short human-readable form for the screen.
impl fmt::Display for WatchEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchEvent::Started { pid, name, .. } => {
                write!(f, "[+] New process detected: {name} (PID {pid})")
            }
            WatchEvent::Exited { pid, name } => write!(f, "[-] Process exited: {name} (PID {pid})"),
            WatchEvent::HighCpu { pid, name, cpu } => {
                write!(f, "[!] High CPU usage: PID {pid} ({name}) {cpu:.1}%")
            }
            WatchEvent::HighMem { pid, name, rss_kb } => write!(
                f,
                "[!] High memory usage: PID {pid} ({name}) {} MB",
                rss_kb / 1024
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TickResult {
    /// Sorted by CPU, then memory, highest first.
    pub rows: Vec<ProcessRow>,
    pub events: Vec<WatchEvent>,
}

struct Tracked {
    sample: ProcessSample,
    high_cpu: bool,
    high_mem: bool,
}

pub struct Watcher {
    cfg: Config,
    procs: HashMap<u32, Tracked>,
    prev_total_ticks: u64,
    /// The first tick only records a baseline (no "new process" spam at startup).
    first_tick: bool,
}

impl Watcher {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            procs: HashMap::new(),
            prev_total_ticks: 0,
            first_tick: true,
        }
    }

    /// Reads /proc and updates the state. This does blocking file I/O.
    pub fn tick(&mut self) -> io::Result<TickResult> {
        let (total_ticks, cores) = linux::read_cpu_total()?;
        let samples = linux::list_pids()?
            .into_iter()
            .filter_map(linux::read_process)
            .collect();
        Ok(self.process_samples(total_ticks, cores, samples))
    }

    fn matches_filter(&self, s: &ProcessSample) -> bool {
        match &self.cfg.filter {
            None => true,
            Some(f) => s.name.to_lowercase().contains(f) || s.cmdline.to_lowercase().contains(f),
        }
    }

    /// The pure part of a tick, kept separate so it can be unit-tested.
    fn process_samples(
        &mut self,
        total_ticks: u64,
        cores: usize,
        samples: Vec<ProcessSample>,
    ) -> TickResult {
        let elapsed_ticks = total_ticks.saturating_sub(self.prev_total_ticks);
        let mut next = HashMap::with_capacity(samples.len());
        let mut rows = Vec::with_capacity(samples.len());
        let mut events = Vec::new();
        let report = !self.first_tick;

        for sample in samples {
            if !self.matches_filter(&sample) {
                continue;
            }

            // Same PID but different start time = the old process exited and the PID was reused.
            let prev = match self.procs.remove(&sample.pid) {
                Some(t) if t.sample.start_time == sample.start_time => Some(t),
                Some(old) => {
                    events.push(WatchEvent::Exited { pid: old.sample.pid, name: old.sample.name });
                    None
                }
                None => None,
            };

            let (cpu, mut high_cpu, mut high_mem) = match &prev {
                Some(t) => {
                    let used = sample.cpu_ticks.saturating_sub(t.sample.cpu_ticks);
                    // Like `top`: 100% = one fully busy core.
                    let cpu = if elapsed_ticks == 0 {
                        0.0
                    } else {
                        used as f64 / elapsed_ticks as f64 * cores as f64 * 100.0
                    };
                    (cpu, t.high_cpu, t.high_mem)
                }
                None => {
                    if report {
                        events.push(WatchEvent::Started {
                            pid: sample.pid,
                            ppid: sample.ppid,
                            name: sample.name.clone(),
                            cmdline: sample.cmdline.clone(),
                            exe: sample.exe.clone(),
                        });
                    }
                    (0.0, false, false)
                }
            };

            if cpu >= self.cfg.cpu_threshold {
                if !high_cpu && report {
                    events.push(WatchEvent::HighCpu { pid: sample.pid, name: sample.name.clone(), cpu });
                }
                high_cpu = true;
            } else if cpu < self.cfg.cpu_threshold * HYSTERESIS {
                high_cpu = false;
            }

            // Unlike the other alerts this also fires on the first tick, so memory
            // hogs that were already running when the watcher started get flagged.
            if let Some(limit) = self.cfg.mem_threshold_kb {
                if sample.rss_kb >= limit {
                    if !high_mem {
                        events.push(WatchEvent::HighMem {
                            pid: sample.pid,
                            name: sample.name.clone(),
                            rss_kb: sample.rss_kb,
                        });
                    }
                    high_mem = true;
                } else if (sample.rss_kb as f64) < limit as f64 * HYSTERESIS {
                    high_mem = false;
                }
            }

            rows.push(ProcessRow {
                pid: sample.pid,
                name: sample.name.clone(),
                state: sample.state,
                cpu_percent: cpu,
                rss_kb: sample.rss_kb,
            });
            next.insert(sample.pid, Tracked { sample, high_cpu, high_mem });
        }

        // Anything left from the previous tick is gone now.
        if report {
            let mut gone: Vec<_> = self.procs.drain().map(|(_, t)| t.sample).collect();
            gone.sort_by_key(|s| s.pid);
            events.extend(gone.into_iter().map(|s| WatchEvent::Exited { pid: s.pid, name: s.name }));
        }

        self.procs = next;
        self.prev_total_ticks = total_ticks;
        self.first_tick = false;

        rows.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(Ordering::Equal)
                .then(b.rss_kb.cmp(&a.rss_kb))
        });

        TickResult { rows, events }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, name: &str, cpu_ticks: u64, start_time: u64, rss_kb: u64) -> ProcessSample {
        ProcessSample {
            pid,
            ppid: 1,
            name: name.to_string(),
            state: 'S',
            cpu_ticks,
            start_time,
            rss_kb,
            cmdline: format!("/usr/bin/{name}"),
            exe: Some(format!("/usr/bin/{name}")),
        }
    }

    fn watcher() -> Watcher {
        Watcher::new(Config { cpu_threshold: 50.0, mem_threshold_kb: Some(500_000), filter: None })
    }

    #[test]
    fn first_tick_is_a_silent_baseline() {
        let mut w = watcher();
        let r = w.process_samples(1000, 1, vec![proc(1, "init", 0, 1, 10)]);
        assert!(r.events.is_empty());
        assert_eq!(r.rows.len(), 1);
    }

    #[test]
    fn existing_memory_hog_is_flagged_on_first_tick() {
        let mut w = watcher();
        let r = w.process_samples(1000, 1, vec![proc(1, "java", 0, 1, 900_000)]);
        assert!(matches!(r.events[..], [WatchEvent::HighMem { pid: 1, .. }]));
    }

    #[test]
    fn detects_new_and_exited_processes() {
        let mut w = watcher();
        w.process_samples(1000, 1, vec![proc(1, "init", 0, 1, 10), proc(50, "bash", 0, 5, 10)]);
        let r = w.process_samples(2000, 1, vec![proc(1, "init", 0, 1, 10), proc(77, "nginx", 0, 9, 10)]);

        assert!(r.events.iter().any(|e| matches!(e, WatchEvent::Started { pid: 77, .. })));
        assert!(r.events.iter().any(|e| matches!(e, WatchEvent::Exited { pid: 50, .. })));
        assert_eq!(r.events.len(), 2);
    }

    #[test]
    fn reused_pid_counts_as_exit_plus_start() {
        let mut w = watcher();
        w.process_samples(1000, 1, vec![proc(42, "old", 0, 100, 10)]);
        let r = w.process_samples(2000, 1, vec![proc(42, "new", 0, 200, 10)]);
        assert_eq!(
            r.events,
            vec![
                WatchEvent::Exited { pid: 42, name: "old".into() },
                WatchEvent::Started {
                    pid: 42,
                    ppid: 1,
                    name: "new".into(),
                    cmdline: "/usr/bin/new".into(),
                    exe: Some("/usr/bin/new".into()),
                },
            ]
        );
    }

    #[test]
    fn computes_cpu_and_alerts_once() {
        let mut w = watcher();
        w.process_samples(1000, 2, vec![proc(10, "firefox", 0, 1, 10)]);

        // 1000 total ticks elapsed across 2 cores; process used 400 -> 80% of a core.
        let r = w.process_samples(2000, 2, vec![proc(10, "firefox", 400, 1, 10)]);
        assert!((r.rows[0].cpu_percent - 80.0).abs() < 1e-9);
        assert!(matches!(r.events[..], [WatchEvent::HighCpu { pid: 10, .. }]));

        // Still high -> no repeated alert.
        let r = w.process_samples(3000, 2, vec![proc(10, "firefox", 800, 1, 10)]);
        assert!(r.events.is_empty());

        // Drops well below, then spikes again -> new alert.
        w.process_samples(4000, 2, vec![proc(10, "firefox", 810, 1, 10)]);
        let r = w.process_samples(5000, 2, vec![proc(10, "firefox", 1300, 1, 10)]);
        assert_eq!(r.events.len(), 1);
    }

    #[test]
    fn alerts_on_high_memory() {
        let mut w = watcher();
        w.process_samples(1000, 1, vec![proc(5, "code", 0, 1, 100)]);
        let r = w.process_samples(2000, 1, vec![proc(5, "code", 0, 1, 600_000)]);
        assert!(matches!(r.events[..], [WatchEvent::HighMem { pid: 5, rss_kb: 600_000, .. }]));
    }

    #[test]
    fn filter_limits_watched_processes() {
        let mut w = Watcher::new(Config {
            cpu_threshold: 50.0,
            mem_threshold_kb: None,
            filter: Some("nginx".into()),
        });
        let r = w.process_samples(1000, 1, vec![proc(1, "init", 0, 1, 10), proc(2, "nginx", 0, 1, 10)]);
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0].name, "nginx");
    }

    #[test]
    fn display_matches_spec_format() {
        let e = WatchEvent::HighCpu { pid: 1024, name: "firefox".into(), cpu: 92.0 };
        assert_eq!(e.to_string(), "[!] High CPU usage: PID 1024 (firefox) 92.0%");
    }
}
