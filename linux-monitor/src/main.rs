mod display;
mod linux;
mod monitor;

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use clap::Parser;

use monitor::Monitor;

/// Linux system information monitor — reads CPU, memory, uptime, kernel and
/// process data straight from /proc.
#[derive(Parser, Debug)]
#[command(name = "linux-monitor", version, about)]
struct Cli {
    /// Open the interactive dashboard (press q to quit)
    #[arg(short, long)]
    tui: bool,

    /// Keep refreshing the plain-text report (Ctrl+C to quit)
    #[arg(short, long, conflicts_with = "tui")]
    watch: bool,

    /// Refresh interval in milliseconds
    #[arg(short, long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(100..))]
    interval: u64,

    /// List the top N processes by memory (default 20 when given without a value)
    #[arg(short, long, num_args = 0..=1, default_missing_value = "20")]
    processes: Option<usize>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if !linux::proc_available() {
        eprintln!("error: /proc not found — this tool only runs on Linux (try WSL on Windows).");
        return ExitCode::FAILURE;
    }

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> std::io::Result<()> {
    let interval = Duration::from_millis(cli.interval);
    let mut monitor = Monitor::new()?;

    if cli.tui {
        return display::run_tui(&mut monitor, interval);
    }

    // CPU usage needs two readings, so wait briefly before the first report.
    thread::sleep(interval.min(Duration::from_millis(500)));

    loop {
        let snapshot = monitor.sample()?;
        if cli.watch {
            // Clear the screen and move the cursor home.
            print!("\x1B[2J\x1B[H");
        }
        display::print_report(&snapshot, cli.processes);

        if !cli.watch {
            return Ok(());
        }
        thread::sleep(interval);
    }
}
