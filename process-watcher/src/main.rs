mod display;
mod linux;
mod logger;
mod monitor;

use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use chrono::{DateTime, Local};
use clap::Parser;
use tokio::time::MissedTickBehavior;

use display::ScreenInfo;
use logger::ActivityLog;
use monitor::{Config, WatchEvent, Watcher};

/// How many recent events stay on screen.
const MAX_RECENT_EVENTS: usize = 10;

/// Real-time Linux process watcher — reads /proc/[PID]/{stat,status,cmdline,exe},
/// detects new/exited processes, flags heavy CPU/RAM users and logs the activity.
#[derive(Parser, Debug)]
#[command(name = "process-watcher", version, about)]
struct Cli {
    /// Refresh interval in milliseconds
    #[arg(short, long, default_value_t = 2000, value_parser = clap::value_parser!(u64).range(100..))]
    interval: u64,

    /// CPU % that triggers a high-CPU alert (100 = one full core)
    #[arg(short, long, default_value_t = 50.0)]
    cpu_threshold: f64,

    /// Resident memory in MB that triggers a high-memory alert
    #[arg(short, long)]
    mem_threshold: Option<u64>,

    /// Number of processes shown in the table
    #[arg(short = 'n', long, default_value_t = 15)]
    top: usize,

    /// Log file for process activity
    #[arg(short, long, default_value = "process_watcher.log")]
    log: PathBuf,

    /// Only watch processes whose name or command line contains this text
    #[arg(short, long)]
    filter: Option<String>,

    /// Don't draw the table; only print events (handy for piping)
    #[arg(short, long)]
    quiet: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    if !linux::proc_available() {
        eprintln!("error: /proc not found — this tool only runs on Linux (try WSL on Windows).");
        return ExitCode::FAILURE;
    }

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> io::Result<()> {
    let mut log = ActivityLog::open(&cli.log).await?;
    log.write(&format!(
        "watcher started (interval={}ms, cpu-threshold={}%, mem-threshold={}, filter={})",
        cli.interval,
        cli.cpu_threshold,
        cli.mem_threshold.map_or("off".into(), |m| format!("{m}MB")),
        cli.filter.as_deref().unwrap_or("none"),
    ))
    .await?;

    let mut watcher = Watcher::new(Config {
        cpu_threshold: cli.cpu_threshold,
        mem_threshold_kb: cli.mem_threshold.map(|mb| mb * 1024),
        filter: cli.filter.as_ref().map(|f| f.to_lowercase()),
    });

    let mut recent: VecDeque<(DateTime<Local>, WatchEvent)> = VecDeque::with_capacity(MAX_RECENT_EVENTS);
    let mut ticker = tokio::time::interval(Duration::from_millis(cli.interval));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    if cli.quiet {
        println!("Watching processes (Ctrl+C to quit)...");
    }

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Reading hundreds of /proc files is blocking I/O, so run it off the async runtime.
                let (w, result) = tokio::task::spawn_blocking(move || {
                    let result = watcher.tick();
                    (watcher, result)
                })
                .await
                .map_err(io::Error::other)?;
                watcher = w;
                let result = result?;

                let now = Local::now();
                for event in result.events {
                    log.write(&event.log_line()).await?;
                    if cli.quiet {
                        println!("{} {event}", now.format("[%H:%M:%S]"));
                    }
                    if recent.len() == MAX_RECENT_EVENTS {
                        recent.pop_front();
                    }
                    recent.push_back((now, event));
                }

                if !cli.quiet {
                    let screen = display::render(
                        &result.rows,
                        &recent,
                        &ScreenInfo {
                            total_processes: result.rows.len(),
                            top: cli.top,
                            interval_ms: cli.interval,
                            log_path: &cli.log,
                        },
                    );
                    let mut stdout = io::stdout().lock();
                    stdout.write_all(screen.as_bytes())?;
                    stdout.flush()?;
                }
            }
            _ = &mut ctrl_c => break,
        }
    }

    log.write("watcher stopped").await?;
    println!("\nStopped. Activity saved to {}", cli.log.display());
    Ok(())
}
