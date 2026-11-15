//! Combines the raw /proc readers into a single snapshot of the system.

use std::io;

use crate::linux::{self, CpuTimes, MemInfo, ProcessInfo};

/// Everything shown on one screen refresh.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub kernel: String,
    pub cpu_usage: f64,
    pub cpu_count: usize,
    pub mem: MemInfo,
    pub uptime_secs: f64,
    /// Sorted by resident memory, largest first.
    pub processes: Vec<ProcessInfo>,
}

/// Holds the state needed between samples (CPU usage is a delta of two readings).
pub struct Monitor {
    kernel: String,
    cpu_count: usize,
    prev_cpu: CpuTimes,
}

impl Monitor {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            kernel: linux::read_kernel_version()?,
            cpu_count: linux::read_cpu_count()?,
            prev_cpu: linux::read_cpu_times()?,
        })
    }

    /// Takes a new reading. CPU usage covers the time since the previous call
    /// (or since `new()` for the first call).
    pub fn sample(&mut self) -> io::Result<Snapshot> {
        let cpu = linux::read_cpu_times()?;
        let cpu_usage = cpu.usage_percent_since(&self.prev_cpu);
        self.prev_cpu = cpu;

        let mut processes = linux::list_processes()?;
        processes.sort_by(|a, b| b.rss_kb.cmp(&a.rss_kb).then(a.pid.cmp(&b.pid)));

        Ok(Snapshot {
            kernel: self.kernel.clone(),
            cpu_usage,
            cpu_count: self.cpu_count,
            mem: linux::read_meminfo()?,
            uptime_secs: linux::read_uptime()?,
            processes,
        })
    }
}
