use color_eyre::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal,
    style::Style,
    text::{Line, Span},
    widgets::TableState,
};

use crate::lsof::{self, KillOutcome, LsofEntry};
use crate::ui::Colors;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AppMode {
    ProcessList,
    ConfirmKill,
    Search,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SortBy {
    Port,
    Pid,
    User,
    Command,
    Memory,
    StartTime,
    Protocol,
}

#[derive(Debug)]
pub struct App {
    pub(crate) running: bool,
    pub(crate) scanner: lsof::Scanner,
    pub(crate) processes: Vec<lsof::LsofEntry>,
    pub(crate) filtered_processes: Vec<lsof::LsofEntry>,
    pub(crate) status_message: Option<Line<'static>>,
    pub(crate) loading_message: Option<String>,
    pub(crate) mode: AppMode,
    pub(crate) selected_index: usize,
    pub(crate) table_state: TableState,
    pub(crate) search_query: String,
    pub(crate) sort_by: SortBy,
    pub(crate) sort_ascending: bool,
    pub(crate) loading_animation_frame: usize,
    pub(crate) show_restricted: bool,
    /// Snapshot of the process the ConfirmKill prompt is about. The live
    /// table keeps refreshing underneath the prompt, so the selection
    /// index alone could silently come to point at a different process
    /// between "Enter" and "y".
    pub(crate) pending_kill: Option<LsofEntry>,
}

impl Default for App {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            running: false,
            scanner: lsof::Scanner::default(),
            processes: Vec::new(),
            filtered_processes: Vec::new(),
            status_message: None,
            loading_message: None,
            mode: AppMode::ProcessList,
            selected_index: 0,
            table_state,
            search_query: String::new(),
            sort_by: SortBy::Port,
            sort_ascending: false,
            loading_animation_frame: 0,
            show_restricted: false,
            pending_kill: None,
        }
    }
}

/// Build a success status line with port/command/pid colored to match
/// their table-column hues, so the eye picks each piece out instantly.
/// `verb_color` carries the outcome semantics (green=killed,
/// orange=force-killed).
fn kill_status_line(verb: &str, verb_color: ratatui::style::Color, p: &LsofEntry) -> Line<'static> {
    let dim = Style::default().fg(Colors::TEXT_TERTIARY);
    Line::from(vec![
        Span::styled("✓ ", Style::default().fg(Colors::SUCCESS).bold()),
        Span::styled(format!("{} ", verb), Style::default().fg(verb_color).bold()),
        Span::styled(
            format!(":{}", p.port),
            Style::default().fg(Colors::PORT_HUE).bold(),
        ),
        Span::styled("  ", dim),
        Span::styled(p.command.clone(), Style::default().fg(Colors::COMMAND_HUE)),
        Span::styled("  pid ", dim),
        Span::styled(p.pid.clone(), Style::default().fg(Colors::PID_HUE).bold()),
    ])
}

/// A kill that didn't work: red ✗ plus the explanation. Shown in the same
/// status band as successes (it survives the auto-refresh and clears on
/// the next keypress).
fn kill_failure_line(message: String) -> Line<'static> {
    Line::from(vec![
        Span::styled("✗ ", Style::default().fg(Colors::DANGER).bold()),
        Span::styled(message, Style::default().fg(Colors::DANGER)),
    ])
}

impl App {
    pub fn new() -> Self {
        Self {
            loading_message: Some("Initializing port scanner...".to_string()),
            ..Default::default()
        }
    }

    pub fn refresh_processes(&mut self) {
        self.processes = self.scanner.scan();
        self.apply_filter_and_sort();

        if !self.search_query.is_empty()
            && self.filtered_processes.is_empty()
            && self.mode != AppMode::Search
        {
            self.search_query.clear();
            self.apply_filter_and_sort();
        }

        self.loading_message = None;
        if self.selected_index >= self.filtered_processes.len() {
            self.selected_index = 0;
        }
        self.table_state
            .select(if self.filtered_processes.is_empty() {
                None
            } else {
                Some(self.selected_index)
            });
    }

    pub(crate) fn apply_filter_and_sort(&mut self) {
        // ASCII case-folding, matching highlight_matching_text in ui.rs,
        // so every row this filter keeps also gets its match underlined.
        let query = self.search_query.to_ascii_lowercase();
        let has_query = !query.is_empty();

        self.filtered_processes = self
            .processes
            .iter()
            .filter(|p| {
                if !self.show_restricted && !p.is_killable() {
                    return false;
                }
                if !has_query {
                    return true;
                }
                p.command.to_ascii_lowercase().contains(&query)
                    || p.user.to_ascii_lowercase().contains(&query)
                    || p.local_addr.to_ascii_lowercase().contains(&query)
                    || p.port.to_string().contains(&query)
                    || p.pid.contains(&query)
                    || p.cwd
                        .as_deref()
                        .is_some_and(|c| c.to_ascii_lowercase().contains(&query))
            })
            .cloned()
            .collect();

        self.filtered_processes.sort_by(|a, b| {
            let comparison = match self.sort_by {
                SortBy::Port => a.port.cmp(&b.port),
                SortBy::Pid => a
                    .pid
                    .parse::<u32>()
                    .unwrap_or(u32::MAX)
                    .cmp(&b.pid.parse::<u32>().unwrap_or(u32::MAX)),
                SortBy::User => a.user.cmp(&b.user),
                SortBy::Command => a.command.cmp(&b.command),
                SortBy::Memory => a
                    .memory_mb
                    .partial_cmp(&b.memory_mb)
                    .unwrap_or(std::cmp::Ordering::Equal),
                SortBy::StartTime => match (&a.start_time, &b.start_time) {
                    (Some(at), Some(bt)) => at.cmp(bt),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                },
                SortBy::Protocol => a.protocol.cmp(b.protocol),
            };

            if self.sort_ascending {
                comparison
            } else {
                comparison.reverse()
            }
        });
    }

    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        use std::time::{Duration, Instant};
        self.running = true;

        terminal.draw(|frame| self.render(frame))?;

        self.refresh_processes();

        let refresh_interval = Duration::from_secs(1);
        let animation_interval = Duration::from_millis(100);
        let mut last_refresh = Instant::now();
        let mut last_animation = Instant::now();
        let mut needs_redraw = true;

        while self.running {
            if needs_redraw {
                terminal.draw(|frame| self.render(frame))?;
                needs_redraw = false;
            }

            // The 100ms spinner tick only matters while a loading message
            // is up; when idle, sleep the full stretch to the next refresh
            // instead of waking (and redrawing) ten times a second.
            let until_refresh = refresh_interval.saturating_sub(last_refresh.elapsed());
            let timeout = if self.mode == AppMode::ConfirmKill {
                // Refreshes are frozen, so nothing periodic runs — a zero
                // `until_refresh` here would spin the poll loop. Just wait
                // for a key in comfortable stretches.
                refresh_interval
            } else if self.loading_message.is_some() {
                until_refresh.min(animation_interval.saturating_sub(last_animation.elapsed()))
            } else {
                until_refresh
            };

            if event::poll(timeout)? {
                self.handle_crossterm_events()?;
                needs_redraw = true;
            }

            // Freeze the list while the ConfirmKill prompt is up: the user
            // is deciding based on what's on screen, so nothing may reorder
            // under them. Paired with the ConfirmKill timeout branch above;
            // the mode is deliberately re-read here because the key event
            // just handled may have entered or left the prompt.
            if self.mode != AppMode::ConfirmKill && last_refresh.elapsed() >= refresh_interval {
                self.refresh_processes();
                last_refresh = Instant::now();
                needs_redraw = true;
            }

            if self.loading_message.is_some() && last_animation.elapsed() >= animation_interval {
                self.loading_animation_frame = (self.loading_animation_frame + 1) % 10;
                last_animation = Instant::now();
                needs_redraw = true;
            }
        }
        Ok(())
    }

    fn handle_crossterm_events(&mut self) -> Result<()> {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key),
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
            _ => {}
        }
        Ok(())
    }

    fn on_key_event(&mut self, key: KeyEvent) {
        // Ctrl+C quits from every mode — never trapped by a prompt,
        // never typed into the search box.
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        {
            self.quit();
            return;
        }

        if self.mode == AppMode::ProcessList {
            self.status_message = None;
        }

        match self.mode {
            AppMode::ProcessList => match (key.modifiers, key.code) {
                (_, KeyCode::Esc) => {
                    if !self.search_query.is_empty() {
                        self.search_query.clear();
                        self.apply_filter_and_sort();
                        self.selected_index = 0;
                        self.table_state
                            .select(if self.filtered_processes.is_empty() {
                                None
                            } else {
                                Some(0)
                            });
                    } else {
                        self.quit();
                    }
                }
                (_, KeyCode::Char('q')) => self.quit(),
                (_, KeyCode::Char('r') | KeyCode::Char('R')) => {
                    self.refresh_processes();
                }
                (_, KeyCode::Up) => {
                    self.select_previous();
                }
                (_, KeyCode::Down) => {
                    self.select_next();
                }
                (_, KeyCode::Enter) => {
                    self.enter_confirm_mode();
                }
                (_, KeyCode::Char('/')) => {
                    self.enter_search_mode();
                }
                (_, KeyCode::Char('s') | KeyCode::Char('S')) => {
                    self.cycle_sort();
                }
                (_, KeyCode::Char('a') | KeyCode::Char('A')) => {
                    self.toggle_restricted();
                }
                // 1-7 mirror the visual column order: PORT, USER, MEM,
                // UPTIME, PROTO, PID, COMMAND.
                (_, KeyCode::Char('1')) => {
                    self.set_sort(SortBy::Port);
                }
                (_, KeyCode::Char('2')) => {
                    self.set_sort(SortBy::User);
                }
                (_, KeyCode::Char('3')) => {
                    self.set_sort(SortBy::Memory);
                }
                (_, KeyCode::Char('4')) => {
                    self.set_sort(SortBy::StartTime);
                }
                (_, KeyCode::Char('5')) => {
                    self.set_sort(SortBy::Protocol);
                }
                (_, KeyCode::Char('6')) => {
                    self.set_sort(SortBy::Pid);
                }
                (_, KeyCode::Char('7')) => {
                    self.set_sort(SortBy::Command);
                }
                (_, KeyCode::Backspace) if !self.search_query.is_empty() => {
                    self.search_query.pop();
                    self.apply_filter_and_sort();
                    self.selected_index = 0;
                    self.table_state
                        .select(if self.filtered_processes.is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                }
                _ => {}
            },
            AppMode::ConfirmKill => match (key.modifiers, key.code) {
                (_, KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter) => {
                    self.confirm_kill()
                }
                (_, KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc) => self.cancel_kill(),
                _ => {}
            },
            AppMode::Search => match (key.modifiers, key.code) {
                (_, KeyCode::Esc) => self.exit_search_mode(),
                (_, KeyCode::Enter) => self.apply_search(),
                (_, KeyCode::Backspace) => {
                    self.search_query.pop();
                    self.apply_filter_and_sort();
                    self.selected_index = 0;
                    self.table_state
                        .select(if self.filtered_processes.is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                }
                // SHIFT is how uppercase arrives; any other chord (Ctrl/Alt
                // combos) is a command, not text — don't type it.
                (m, KeyCode::Char(c))
                    if !m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.search_query.push(c);
                    self.apply_filter_and_sort();
                    self.selected_index = 0;
                    self.table_state
                        .select(if self.filtered_processes.is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                }
                _ => {}
            },
        }
    }

    fn select_previous(&mut self) {
        if !self.filtered_processes.is_empty() {
            if self.selected_index > 0 {
                self.selected_index -= 1;
            } else {
                self.selected_index = self.filtered_processes.len() - 1;
            }
            self.table_state.select(Some(self.selected_index));
        }
    }

    fn select_next(&mut self) {
        if !self.filtered_processes.is_empty() {
            if self.selected_index < self.filtered_processes.len() - 1 {
                self.selected_index += 1;
            } else {
                self.selected_index = 0;
            }
            self.table_state.select(Some(self.selected_index));
        }
    }

    fn enter_confirm_mode(&mut self) {
        let Some(selected) = self.filtered_processes.get(self.selected_index) else {
            return;
        };
        if !selected.is_killable() {
            let dim = Style::default().fg(Colors::TEXT_TERTIARY);
            self.status_message = Some(Line::from(vec![
                Span::styled("Cannot kill ", Style::default().fg(Colors::DANGER).bold()),
                Span::styled(
                    format!(":{}", selected.port),
                    Style::default().fg(Colors::PORT_HUE).bold(),
                ),
                Span::styled(" — owned by ", dim),
                Span::styled(selected.user.clone(), Style::default().fg(Colors::USER_HUE)),
                Span::styled(", re-run with sudo", dim),
            ]));
            return;
        }
        self.pending_kill = Some(selected.clone());
        self.mode = AppMode::ConfirmKill;
    }

    fn enter_search_mode(&mut self) {
        self.mode = AppMode::Search;
    }

    fn exit_search_mode(&mut self) {
        self.mode = AppMode::ProcessList;
        self.search_query.clear();
        self.apply_filter_and_sort();
        self.selected_index = 0;
        self.table_state
            .select(if self.filtered_processes.is_empty() {
                None
            } else {
                Some(0)
            });
    }

    fn apply_search(&mut self) {
        self.mode = AppMode::ProcessList;
        self.apply_filter_and_sort();
        self.selected_index = 0;
        self.table_state
            .select(if self.filtered_processes.is_empty() {
                None
            } else {
                Some(0)
            });
    }

    fn toggle_restricted(&mut self) {
        self.show_restricted = !self.show_restricted;
        self.apply_filter_and_sort();
        self.selected_index = 0;
        self.table_state
            .select(if self.filtered_processes.is_empty() {
                None
            } else {
                Some(0)
            });
    }

    pub(crate) fn restricted_hidden_count(&self) -> usize {
        if self.show_restricted {
            0
        } else {
            self.processes.iter().filter(|p| !p.is_killable()).count()
        }
    }

    fn cycle_sort(&mut self) {
        // Cycle follows the visual column order:
        // PORT → USER → MEM → UPTIME → PROTO → PID → COMMAND → PORT.
        self.sort_by = match self.sort_by {
            SortBy::Port => SortBy::User,
            SortBy::User => SortBy::Memory,
            SortBy::Memory => SortBy::StartTime,
            SortBy::StartTime => SortBy::Protocol,
            SortBy::Protocol => SortBy::Pid,
            SortBy::Pid => SortBy::Command,
            SortBy::Command => SortBy::Port,
        };
        self.apply_filter_and_sort();
    }

    fn set_sort(&mut self, sort_by: SortBy) {
        if self.sort_by == sort_by {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_by = sort_by;
            self.sort_ascending = false;
        }
        self.apply_filter_and_sort();
    }

    fn confirm_kill(&mut self) {
        self.mode = AppMode::ProcessList;
        // Kill the snapshotted process the user actually confirmed — never
        // whatever the current selection index happens to point at.
        let Some(process) = self.pending_kill.take() else {
            return;
        };

        self.status_message =
            match lsof::kill_process_verified(&process.pid, process.starttime_ticks) {
                Ok(KillOutcome::Terminated) => {
                    Some(kill_status_line("Killed", Colors::SUCCESS, &process))
                }
                Ok(KillOutcome::ForceKilled) => {
                    let mut line = kill_status_line("Force-killed", Colors::WARNING, &process);
                    line.spans.push(Span::styled(
                        "  (ignored SIGTERM)",
                        Style::default().fg(Colors::TEXT_TERTIARY),
                    ));
                    Some(line)
                }
                Ok(KillOutcome::StillAlive) => Some(kill_failure_line(format!(
                    "{} ({}) is still alive after SIGKILL — likely a kernel-stuck process",
                    process.command, process.pid
                ))),
                Err(e) => Some(kill_failure_line(format!(
                    "Failed to signal {} ({}): {}",
                    process.command, process.pid, e
                ))),
            };

        self.refresh_processes();
    }

    fn cancel_kill(&mut self) {
        self.pending_kill = None;
        self.mode = AppMode::ProcessList;
    }

    fn quit(&mut self) {
        self.running = false;
    }
}
