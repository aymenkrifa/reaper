//! Non-interactive entry points: argument parsing plus the `list` and
//! `kill` subcommands. Both reuse the same scanner and verified kill path
//! as the TUI, so a process looks and dies the same way from either side.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::lsof::{self, KillOutcome, LsofEntry};

const AFTER_HELP: &str = "\
Keys (inside the TUI):
  ↑/↓ navigate • ⏎ kill (with confirmation) • / search
  s or 1-7 sort • a show restricted • r refresh • q/Esc quit

Run with sudo to see and kill other users' listeners.
Docs: https://reaper.aymenkrifa.com";

#[derive(Parser)]
#[command(
    name = "reaper",
    version,
    about = "reaper — a linux tui for listing and killing listening ports",
    after_help = AFTER_HELP,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Open the TUI with this search already applied (matches port,
    /// user, address or command)
    pub query: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print listening ports and exit
    #[command(visible_alias = "ls")]
    List {
        /// Only show these ports
        #[arg(short, long = "port", value_name = "PORT")]
        ports: Vec<u16>,
        /// Only show listeners owned by this user
        #[arg(short, long)]
        user: Option<String>,
        /// Include other users' listeners whose process can't be inspected
        #[arg(short, long)]
        all: bool,
        /// Print JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Kill whatever is listening on the given ports (SIGTERM, then SIGKILL)
    Kill {
        #[arg(required = true, value_name = "PORT")]
        ports: Vec<u16>,
        /// Don't ask for confirmation
        #[arg(short, long)]
        yes: bool,
    },
    /// Download and install the latest release
    Update,
}

/// `reaper list`: the TUI's table as plain text (or JSON), sorted by port.
pub fn list(ports: &[u16], user: Option<&str>, all: bool, json: bool) -> ExitCode {
    let mut entries: Vec<LsofEntry> = lsof::Scanner::default()
        .scan()
        .into_iter()
        .filter(|p| all || p.is_killable())
        .filter(|p| ports.is_empty() || ports.contains(&p.port))
        .filter(|p| user.is_none_or(|u| p.user == u))
        .collect();
    entries.sort_by(|a, b| a.port.cmp(&b.port).then_with(|| a.pid.cmp(&b.pid)));

    // An empty JSON array is a useful answer for scripts; a header with no
    // rows under it isn't, so say so on stderr instead.
    if entries.is_empty() && !json {
        eprintln!("no matching listeners");
        return if ports.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    let out = if json {
        let rows: Vec<serde_json::Value> = entries.iter().map(entry_json).collect();
        // Pretty-printing a Vec of plain JSON values can't fail.
        serde_json::to_string_pretty(&rows).unwrap_or_default() + "\n"
    } else {
        render_table(&entries)
    };

    // A closed pipe (`reaper list | head`) isn't an error worth reporting.
    let _ = io::stdout().lock().write_all(out.as_bytes());

    // Asking for specific ports and finding none is a miss, like grep.
    if entries.is_empty() && !ports.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn entry_json(p: &LsofEntry) -> serde_json::Value {
    let killable = p.is_killable();
    serde_json::json!({
        "port": p.port,
        "protocol": p.protocol,
        "address": p.local_addr,
        "pid": p.pid.parse::<u32>().ok(),
        "user": p.user,
        "command": p.command,
        "cwd": p.cwd,
        "memory_mb": if killable { Some((p.memory_mb * 10.0).round() / 10.0) } else { None },
        "uptime_secs": p.start_time.and_then(|t| t.elapsed().ok()).map(|d| d.as_secs()),
        "killable": killable,
    })
}

fn render_table(entries: &[LsofEntry]) -> String {
    let header = ["PORT", "USER", "MEM", "UPTIME", "PROTO", "PID", "COMMAND"];
    let rows: Vec<[String; 7]> = entries
        .iter()
        .map(|p| {
            let killable = p.is_killable();
            [
                format!(":{}", p.port),
                p.user.clone(),
                if killable {
                    p.get_memory_display()
                } else {
                    "—".into()
                },
                if p.start_time.is_some() {
                    p.get_relative_time()
                } else {
                    "—".into()
                },
                p.protocol.to_string(),
                p.pid.clone(),
                p.command.clone(),
            ]
        })
        .collect();

    // Size every column but COMMAND to its widest cell; COMMAND is last
    // and printed in full so nothing is lost when piping to grep.
    let mut widths = header.map(str::len);
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }

    let mut out = String::new();
    let mut push_row = |cells: [&str; 7]| {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i == cells.len() - 1 {
                line.push_str(cell);
            } else {
                let pad = widths[i] - cell.chars().count();
                line.push_str(cell);
                line.push_str(&" ".repeat(pad + 2));
            }
        }
        out.push_str(&line);
        out.push('\n');
    };
    push_row(header);
    for row in &rows {
        push_row(row.each_ref().map(String::as_str));
    }
    out
}

/// One process to kill, with every requested port it holds — a server
/// listening on both IPv4 and IPv6, or on two requested ports, is still
/// one process and gets one prompt line and one kill.
struct Target {
    entry: LsofEntry,
    ports: Vec<u16>,
}

/// `reaper kill <port>...`: resolve each port to its process, confirm,
/// then kill through the same verified path as the TUI. Exits non-zero if
/// any port had nothing killable on it or any kill failed.
pub fn kill(ports: &[u16], yes: bool) -> ExitCode {
    let entries = lsof::Scanner::default().scan();
    let (targets, mut ok) = resolve_targets(&entries, ports);

    if targets.is_empty() {
        return ExitCode::FAILURE;
    }

    for t in &targets {
        println!("  {}", describe(t));
    }

    if !yes && !confirm(targets.len()) {
        eprintln!("aborted — nothing was killed (pass -y to skip this prompt)");
        return ExitCode::FAILURE;
    }

    for t in &targets {
        let what = describe(t);
        match lsof::kill_process_verified(&t.entry.pid, t.entry.starttime_ticks) {
            Ok(KillOutcome::Terminated) => println!("✓ killed {what}"),
            Ok(KillOutcome::ForceKilled) => {
                println!("✓ force-killed {what} (ignored SIGTERM)")
            }
            Ok(KillOutcome::StillAlive) => {
                eprintln!("✗ {what} is still alive after SIGKILL — likely a kernel-stuck process");
                ok = false;
            }
            Err(e) => {
                let hint = if e.raw_os_error() == Some(libc::EPERM) {
                    " — it belongs to another user, retry with sudo"
                } else {
                    ""
                };
                eprintln!("✗ failed to kill {what}: {e}{hint}");
                ok = false;
            }
        }
    }

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Map requested ports to distinct killable processes, reporting (on
/// stderr) every port that can't be acted on. The bool is false when any
/// port was skipped.
fn resolve_targets(entries: &[LsofEntry], ports: &[u16]) -> (Vec<Target>, bool) {
    let mut targets: Vec<Target> = Vec::new();
    let mut ok = true;
    for &port in ports {
        let on_port: Vec<&LsofEntry> = entries.iter().filter(|p| p.port == port).collect();
        if on_port.is_empty() {
            eprintln!("nothing is listening on :{port}");
            ok = false;
            continue;
        }
        let mut warned_restricted = false;
        for p in on_port {
            if !p.is_killable() {
                // v4 + v6 sockets of one restricted listener: warn once.
                if !warned_restricted {
                    eprintln!(
                        ":{port} is held by {}, whose process reaper can't see — retry with sudo",
                        p.user
                    );
                    warned_restricted = true;
                }
                ok = false;
                continue;
            }
            match targets.iter_mut().find(|t| t.entry.pid == p.pid) {
                Some(t) if !t.ports.contains(&port) => t.ports.push(port),
                Some(_) => {}
                None => targets.push(Target {
                    entry: p.clone(),
                    ports: vec![port],
                }),
            }
        }
    }
    (targets, ok)
}

fn describe(t: &Target) -> String {
    let ports: Vec<String> = t.ports.iter().map(|p| format!(":{p}")).collect();
    format!(
        "{}  {}  pid {} ({})",
        ports.join(" "),
        t.entry.command,
        t.entry.pid,
        t.entry.user
    )
}

/// Ask on stderr so stdout stays clean. EOF (no terminal, e.g. in a
/// script) counts as "no" — killing requires an explicit yes or `-y`.
fn confirm(count: usize) -> bool {
    let noun = if count == 1 { "process" } else { "processes" };
    eprint!("Kill {count} {noun}? [y/N] ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    if io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn entry(port: u16, pid: &str, command: &str) -> LsofEntry {
        LsofEntry {
            command: command.to_string(),
            pid: pid.to_string(),
            user: "alice".to_string(),
            local_addr: format!("0.0.0.0:{port}"),
            port,
            protocol: "TCP",
            memory_mb: 12.0,
            start_time: None,
            starttime_ticks: Some(1),
            cwd: None,
        }
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn bare_word_is_a_query_but_subcommands_still_parse() {
        let cli = Cli::try_parse_from(["reaper", "vite"]).unwrap();
        assert_eq!(cli.query.as_deref(), Some("vite"));
        assert!(cli.command.is_none());

        let cli = Cli::try_parse_from(["reaper", "kill", "3000", "5173", "-y"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Kill { ref ports, yes: true }) if ports == &[3000, 5173]
        ));

        assert!(Cli::try_parse_from(["reaper", "kill"]).is_err());
        assert!(Cli::try_parse_from(["reaper", "kill", "99999"]).is_err());
    }

    #[test]
    fn resolve_targets_merges_ports_of_one_process() {
        // Same pid on v4 + v6 and on a second requested port → one target.
        let entries = vec![
            entry(5173, "100", "node vite"),
            entry(5173, "100", "node vite"),
            entry(24678, "100", "node vite"),
            entry(8080, "200", "python -m http.server"),
        ];
        let (targets, ok) = resolve_targets(&entries, &[5173, 24678]);
        assert!(ok);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].ports, vec![5173, 24678]);
    }

    #[test]
    fn resolve_targets_flags_missing_and_restricted_ports() {
        let mut restricted = entry(22, "-", "sshd");
        restricted.user = "root".to_string();
        let entries = vec![restricted, entry(8080, "200", "python")];

        let (targets, ok) = resolve_targets(&entries, &[22, 9999, 8080]);
        assert!(!ok);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].entry.pid, "200");
    }

    #[test]
    fn table_keeps_full_command_and_aligns_columns() {
        let long = "node server.js --a-very-long-flag=".to_string() + &"x".repeat(80);
        let table = render_table(&[entry(80, "1", &long), entry(8080, "22", "python")]);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[1].ends_with(&long));
        let col = |l: &str| l.find("TCP").unwrap();
        assert_eq!(col(lines[1]), col(lines[2]));
    }
}
