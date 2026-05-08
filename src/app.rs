use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, widgets::ListState};

use crate::lsof;

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
}

#[derive(Debug)]
pub struct App {
    pub(crate) running: bool,
    pub(crate) processes: Vec<lsof::LsofEntry>,
    pub(crate) filtered_processes: Vec<lsof::LsofEntry>,
    pub(crate) error_message: Option<String>,
    pub(crate) status_message: Option<String>,
    pub(crate) mode: AppMode,
    pub(crate) selected_index: usize,
    pub(crate) list_state: ListState,
    pub(crate) confirm_button_selected: bool,
    pub(crate) search_query: String,
    pub(crate) sort_by: SortBy,
    pub(crate) sort_ascending: bool,
    pub(crate) loading_animation_frame: usize,
}

impl Default for App {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Self {
            running: false,
            processes: Vec::new(),
            filtered_processes: Vec::new(),
            error_message: None,
            status_message: None,
            mode: AppMode::ProcessList,
            selected_index: 0,
            list_state,
            confirm_button_selected: true,
            search_query: String::new(),
            sort_by: SortBy::Port,
            sort_ascending: false,
            loading_animation_frame: 0,
        }
    }
}

pub(crate) fn extract_port(name: &str) -> u32 {
    if let Some(port_part) = name.rsplit(':').next() {
        port_part
            .replace("(LISTEN)", "")
            .trim()
            .parse()
            .unwrap_or(0)
    } else {
        0
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            status_message: Some("Initializing port scanner...".to_string()),
            ..Default::default()
        }
    }

    pub fn refresh_processes(&mut self) {
        match lsof::get_listening_processes() {
            Ok(processes) => {
                self.processes = processes;
                self.apply_filter_and_sort();

                if !self.search_query.is_empty()
                    && self.filtered_processes.is_empty()
                    && self.mode != AppMode::Search
                {
                    self.search_query.clear();
                    self.apply_filter_and_sort();
                }

                self.error_message = None;
                self.status_message = None;
                if self.selected_index >= self.filtered_processes.len() {
                    self.selected_index = 0;
                }
                self.list_state
                    .select(if self.filtered_processes.is_empty() {
                        None
                    } else {
                        Some(self.selected_index)
                    });
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to get processes: {}", e));
                self.status_message = None;
            }
        }
    }

    pub(crate) fn apply_filter_and_sort(&mut self) {
        self.filtered_processes = if self.search_query.is_empty() {
            self.processes.clone()
        } else {
            self.processes
                .iter()
                .filter(|process| {
                    let query_lower = self.search_query.to_lowercase();
                    process.command.to_lowercase().contains(&query_lower)
                        || process.user.to_lowercase().contains(&query_lower)
                        || process.name.to_lowercase().contains(&query_lower)
                        || process.pid.contains(&query_lower)
                })
                .cloned()
                .collect()
        };

        self.filtered_processes.sort_by(|a, b| {
            let comparison = match self.sort_by {
                SortBy::Port => {
                    let port_a = extract_port(&a.name);
                    let port_b = extract_port(&b.name);
                    port_a.cmp(&port_b)
                }
                SortBy::Pid => a
                    .pid
                    .parse::<u32>()
                    .unwrap_or(0)
                    .cmp(&b.pid.parse::<u32>().unwrap_or(0)),
                SortBy::User => a.user.cmp(&b.user),
                SortBy::Command => a.command.cmp(&b.command),
                SortBy::Memory => a
                    .memory_mb
                    .partial_cmp(&b.memory_mb)
                    .unwrap_or(std::cmp::Ordering::Equal),
                SortBy::StartTime => match (&a.start_time, &b.start_time) {
                    (Some(a_time), Some(b_time)) => a_time.cmp(b_time),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                },
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
                        self.list_state
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
                (_, KeyCode::Char('1')) => {
                    self.set_sort(SortBy::Port);
                }
                (_, KeyCode::Char('2')) => {
                    self.set_sort(SortBy::Pid);
                }
                (_, KeyCode::Char('3')) => {
                    self.set_sort(SortBy::User);
                }
                (_, KeyCode::Char('4')) => {
                    self.set_sort(SortBy::Command);
                }
                (_, KeyCode::Char('5')) => {
                    self.set_sort(SortBy::Memory);
                }
                (_, KeyCode::Char('6')) => {
                    self.set_sort(SortBy::StartTime);
                }
                (_, KeyCode::Backspace) if !self.search_query.is_empty() => {
                    self.search_query.pop();
                    self.apply_filter_and_sort();
                    self.selected_index = 0;
                    self.list_state
                        .select(if self.filtered_processes.is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                }
                _ => {}
            },
            AppMode::ConfirmKill => match (key.modifiers, key.code) {
                (_, KeyCode::Char('y') | KeyCode::Char('Y')) => self.confirm_kill(),
                (_, KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc) => self.cancel_kill(),
                (_, KeyCode::Left) => self.confirm_button_selected = true,
                (_, KeyCode::Right) => self.confirm_button_selected = false,
                (_, KeyCode::Enter) => {
                    if self.confirm_button_selected {
                        self.confirm_kill();
                    } else {
                        self.cancel_kill();
                    }
                }
                _ => {}
            },
            AppMode::Search => match (key.modifiers, key.code) {
                (_, KeyCode::Esc) => self.exit_search_mode(),
                (_, KeyCode::Enter) => self.apply_search(),
                (_, KeyCode::Backspace) => {
                    self.search_query.pop();
                    self.apply_filter_and_sort();
                    self.selected_index = 0;
                    self.list_state
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
                    self.list_state
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
            self.list_state.select(Some(self.selected_index));
        }
    }

    fn select_next(&mut self) {
        if !self.filtered_processes.is_empty() {
            if self.selected_index < self.filtered_processes.len() - 1 {
                self.selected_index += 1;
            } else {
                self.selected_index = 0;
            }
            self.list_state.select(Some(self.selected_index));
        }
    }

    fn enter_confirm_mode(&mut self) {
        if !self.filtered_processes.is_empty() {
            self.mode = AppMode::ConfirmKill;
            self.confirm_button_selected = true;
        }
    }

    fn enter_search_mode(&mut self) {
        self.mode = AppMode::Search;
    }

    fn exit_search_mode(&mut self) {
        self.mode = AppMode::ProcessList;
        self.search_query.clear();
        self.apply_filter_and_sort();
        self.selected_index = 0;
        self.list_state
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
        self.list_state
            .select(if self.filtered_processes.is_empty() {
                None
            } else {
                Some(0)
            });
    }

    fn cycle_sort(&mut self) {
        self.sort_by = match self.sort_by {
            SortBy::Port => SortBy::Pid,
            SortBy::Pid => SortBy::User,
            SortBy::User => SortBy::Command,
            SortBy::Command => SortBy::Memory,
            SortBy::Memory => SortBy::StartTime,
            SortBy::StartTime => SortBy::Port,
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
        if let Some(process) = self.filtered_processes.get(self.selected_index) {
            let pid = &process.pid;
            let command = &process.command;

            match lsof::kill_process(pid) {
                Ok(()) => {
                    self.status_message =
                        Some(format!("Successfully killed process {} ({})", command, pid));
                    self.error_message = None;
                    self.mode = AppMode::ProcessList;

                    std::thread::sleep(std::time::Duration::from_millis(500));
                    self.refresh_processes();
                }
                Err(e) => match lsof::force_kill_process(pid) {
                    Ok(()) => {
                        self.status_message =
                            Some(format!("Force killed process {} ({})", command, pid));
                        self.error_message = None;
                        self.mode = AppMode::ProcessList;

                        std::thread::sleep(std::time::Duration::from_millis(500));
                        self.refresh_processes();
                    }
                    Err(force_err) => {
                        self.error_message = Some(format!(
                            "Failed to kill process: {} | Force kill also failed: {}",
                            e, force_err
                        ));
                        self.status_message = None;
                        self.mode = AppMode::ProcessList;
                    }
                },
            }
        }
    }

    fn cancel_kill(&mut self) {
        self.mode = AppMode::ProcessList;
    }

    fn quit(&mut self) {
        self.running = false;
    }
}
