use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::TableState,
    DefaultTerminal,
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
    pub(crate) processes: Vec<lsof::LsofEntry>,
    pub(crate) filtered_processes: Vec<lsof::LsofEntry>,
    pub(crate) error_message: Option<String>,
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
}

impl Default for App {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            running: false,
            processes: Vec::new(),
            filtered_processes: Vec::new(),
            error_message: None,
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
        }
    }
}

/// Build a status line with port/command/pid colored to match their
/// table-column hues, so the eye picks each piece out instantly.
/// `verb_color` carries the outcome semantics (green=killed,
/// orange=force-killed, danger handling lives in error_message).
fn kill_status_line(verb: &str, verb_color: ratatui::style::Color, p: &LsofEntry) -> Line<'static> {
    let dim = Style::default().fg(Colors::TEXT_TERTIARY);
    Line::from(vec![
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

impl App {
    pub fn new() -> Self {
        Self {
            loading_message: Some("Initializing port scanner...".to_string()),
            ..Default::default()
        }
    }

    pub fn refresh_processes(&mut self) {
        self.processes = lsof::get_listening_processes();
        self.apply_filter_and_sort();

        if !self.search_query.is_empty()
            && self.filtered_processes.is_empty()
            && self.mode != AppMode::Search
        {
            self.search_query.clear();
            self.apply_filter_and_sort();
        }

        self.error_message = None;
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
        let query = self.search_query.to_lowercase();
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
                p.command.to_lowercase().contains(&query)
                    || p.user.to_lowercase().contains(&query)
                    || p.local_addr.to_lowercase().contains(&query)
                    || p.port.to_string().contains(&query)
                    || p.pid.contains(&query)
                    || p.cwd
                        .as_deref()
                        .is_some_and(|c| c.to_lowercase().contains(&query))
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

        while self.running {
            let timeout = refresh_interval
                .checked_sub(last_refresh.elapsed())
                .unwrap_or(Duration::from_secs(0))
                .min(
                    animation_interval
                        .checked_sub(last_animation.elapsed())
                        .unwrap_or(Duration::from_secs(0)),
                );

            if crossterm::event::poll(timeout)? {
                self.handle_crossterm_events()?;
            }

            if last_refresh.elapsed() >= refresh_interval {
                self.refresh_processes();
                last_refresh = Instant::now();
            }

            if last_animation.elapsed() >= animation_interval {
                self.loading_animation_frame = (self.loading_animation_frame + 1) % 10;
                last_animation = Instant::now();
            }

            terminal.draw(|frame| self.render(frame))?;
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
                (_, KeyCode::Char('q'))
                | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => self.quit(),
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
                (_, KeyCode::Char(c)) => {
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
        let Some(process) = self.filtered_processes.get(self.selected_index) else {
            return;
        };
        let process = process.clone();
        let pid = process.pid.clone();
        let command = process.command.clone();
        self.mode = AppMode::ProcessList;

        match lsof::kill_process_verified(&pid) {
            Ok(KillOutcome::Terminated) => {
                self.status_message = Some(kill_status_line("Killed", Colors::SUCCESS, &process));
                self.error_message = None;
            }
            Ok(KillOutcome::ForceKilled) => {
                let mut line = kill_status_line("Force-killed", Colors::WARNING, &process);
                line.spans.push(Span::styled(
                    "  (ignored SIGTERM)",
                    Style::default().fg(Colors::TEXT_TERTIARY),
                ));
                self.status_message = Some(line);
                self.error_message = None;
            }
            Ok(KillOutcome::StillAlive) => {
                self.error_message = Some(format!(
                    "{} ({}) is still alive after SIGKILL — likely a kernel-stuck process",
                    command, pid
                ));
                self.status_message = None;
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to signal {} ({}): {}", command, pid, e));
                self.status_message = None;
            }
        }

        self.refresh_processes();
    }

    fn cancel_kill(&mut self) {
        self.mode = AppMode::ProcessList;
    }

    fn quit(&mut self) {
        self.running = false;
    }
}
