//! Output: a plain-text report and an interactive ratatui dashboard.

use std::io;
use std::time::{Duration, Instant};

use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Gauge, Paragraph, Row, Table},
    DefaultTerminal, Frame,
};

use crate::monitor::{Monitor, Snapshot};

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Formats a size given in kilobytes as KB / MB / GB.
pub fn format_kb(kb: u64) -> String {
    const MB: f64 = 1024.0;
    const GB: f64 = 1024.0 * 1024.0;
    let kb_f = kb as f64;
    if kb_f >= GB {
        format!("{:.1} GB", kb_f / GB)
    } else if kb_f >= MB {
        format!("{:.0} MB", kb_f / MB)
    } else {
        format!("{kb} KB")
    }
}

/// Formats seconds as "2 days, 3 hours, 14 minutes".
pub fn format_uptime(secs: f64) -> String {
    let total = secs as u64;
    let (days, hours, minutes) = (total / 86_400, (total % 86_400) / 3_600, (total % 3_600) / 60);
    let unit = |n: u64, name: &str| format!("{n} {name}{}", if n == 1 { "" } else { "s" });

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(unit(days, "day"));
    }
    if hours > 0 {
        parts.push(unit(hours, "hour"));
    }
    if minutes > 0 || parts.is_empty() {
        parts.push(unit(minutes, "minute"));
    }
    parts.join(", ")
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        100.0 * part as f64 / whole as f64
    }
}

// ---------------------------------------------------------------------------
// Plain-text report
// ---------------------------------------------------------------------------

/// Prints the report. `top` = how many processes to list (None = count only).
pub fn print_report(s: &Snapshot, top: Option<usize>) {
    let m = &s.mem;
    println!("====== RUST LINUX MONITOR ======");
    println!();
    println!("Kernel:    {}", s.kernel);
    println!("CPU Usage: {:.1}% ({} cores)", s.cpu_usage, s.cpu_count);
    println!(
        "RAM Usage: {} / {} ({:.0}%)",
        format_kb(m.used_kb()),
        format_kb(m.total_kb),
        percent(m.used_kb(), m.total_kb)
    );
    if m.swap_total_kb > 0 {
        println!(
            "Swap:      {} / {}",
            format_kb(m.swap_used_kb()),
            format_kb(m.swap_total_kb)
        );
    } else {
        println!("Swap:      none");
    }
    println!("Uptime:    {}", format_uptime(s.uptime_secs));
    println!();
    println!("Running Processes: {}", s.processes.len());

    if let Some(n) = top {
        println!();
        println!("{:<8} {:<20} {:<16} {:>8} {:>10}", "PID", "NAME", "STATE", "THREADS", "RAM");
        for p in s.processes.iter().take(n) {
            println!(
                "{:<8} {:<20} {:<16} {:>8} {:>10}",
                p.pid,
                truncate(&p.name, 20),
                truncate(&p.state, 16),
                p.threads,
                format_kb(p.rss_kb)
            );
        }
    }

    println!();
    println!("================================");
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

// ---------------------------------------------------------------------------
// Interactive TUI (ratatui)
// ---------------------------------------------------------------------------

pub fn run_tui(monitor: &mut Monitor, interval: Duration) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = tui_loop(&mut terminal, monitor, interval);
    // Always restore the terminal, even if the loop failed.
    ratatui::restore();
    result
}

fn tui_loop(terminal: &mut DefaultTerminal, monitor: &mut Monitor, interval: Duration) -> io::Result<()> {
    let mut snap = monitor.sample()?;
    let mut last = Instant::now();

    loop {
        terminal.draw(|f| draw(f, &snap, interval))?;

        let timeout = interval.saturating_sub(last.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                {
                    return Ok(());
                }
            }
        }

        if last.elapsed() >= interval {
            snap = monitor.sample()?;
            last = Instant::now();
        }
    }
}

fn gauge_color(pct: f64) -> Color {
    match pct {
        p if p >= 85.0 => Color::Red,
        p if p >= 60.0 => Color::Yellow,
        _ => Color::Green,
    }
}

fn gauge<'a>(title: &'a str, pct: f64, label: String) -> Gauge<'a> {
    Gauge::default()
        .block(Block::bordered().title(title))
        .gauge_style(Style::default().fg(gauge_color(pct)))
        .ratio((pct / 100.0).clamp(0.0, 1.0))
        .label(label)
}

fn draw(f: &mut Frame, s: &Snapshot, interval: Duration) {
    let [header, cpu, ram, swap, procs] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(5),
    ])
    .areas(f.area());

    let info = Paragraph::new(vec![
        Line::from(format!("Kernel: {}    Cores: {}", s.kernel, s.cpu_count)),
        Line::from(format!(
            "Uptime: {}    Processes: {}",
            format_uptime(s.uptime_secs),
            s.processes.len()
        )),
    ])
    .block(
        Block::bordered()
            .title(" RUST LINUX MONITOR ")
            .title_bottom(format!(" q: quit · refresh {} ms ", interval.as_millis())),
    );
    f.render_widget(info, header);

    f.render_widget(gauge(" CPU ", s.cpu_usage, format!("{:.1}%", s.cpu_usage)), cpu);

    let m = &s.mem;
    let ram_pct = percent(m.used_kb(), m.total_kb);
    f.render_widget(
        gauge(
            " RAM ",
            ram_pct,
            format!("{} / {} ({:.0}%)", format_kb(m.used_kb()), format_kb(m.total_kb), ram_pct),
        ),
        ram,
    );

    let swap_pct = percent(m.swap_used_kb(), m.swap_total_kb);
    let swap_label = if m.swap_total_kb == 0 {
        "no swap".to_string()
    } else {
        format!("{} / {}", format_kb(m.swap_used_kb()), format_kb(m.swap_total_kb))
    };
    f.render_widget(gauge(" Swap ", swap_pct, swap_label), swap);

    // Rows that fit inside the borders and header.
    let visible = procs.height.saturating_sub(3) as usize;
    let rows = s.processes.iter().take(visible).map(|p| {
        Row::new(vec![
            p.pid.to_string(),
            p.name.clone(),
            p.state.clone(),
            p.threads.to_string(),
            format_kb(p.rss_kb),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Min(16),
            Constraint::Length(16),
            Constraint::Length(8),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec!["PID", "NAME", "STATE", "THREADS", "RAM"])
            .style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)),
    )
    .block(Block::bordered().title(" Processes (by memory) "));
    f.render_widget(table, procs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(format_kb(512), "512 KB");
        assert_eq!(format_kb(870_400), "850 MB");
        assert_eq!(format_kb(16 * 1024 * 1024), "16.0 GB");
        assert_eq!(format_kb(4_404_019), "4.2 GB");
    }

    #[test]
    fn formats_uptime() {
        assert_eq!(format_uptime(30.0), "0 minutes");
        assert_eq!(format_uptime(43_200.0), "12 hours");
        assert_eq!(format_uptime(90_061.0), "1 day, 1 hour, 1 minute");
    }

    #[test]
    fn truncates_by_chars() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghijkl", 5), "abcd…");
    }
}
