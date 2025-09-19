use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph},
    DefaultTerminal, Frame,
};

mod lsof;

fn extract_port(name: &str) -> u32 {
    if let Some(port_part) = name.split(':').last() {
        port_part
            .replace("(LISTEN)", "")
            .trim()
            .parse()
            .unwrap_or(0)
    } else {
        0
    }
}

fn get_loading_animation(frame: usize) -> &'static str {
    let animations = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    animations[frame % animations.len()]
}

// Enhanced color palette for gruyere-style UI
struct Colors;
impl Colors {
    const ACCENT: Color = Color::Rgb(26, 188, 156); // Cyan accent
    const TEXT_PRIMARY: Color = Color::Rgb(240, 240, 240); // Light gray
    const TEXT_SECONDARY: Color = Color::Rgb(180, 180, 180); // Medium gray
    const TEXT_TERTIARY: Color = Color::Rgb(120, 120, 120); // Darker gray
    const TEXT_MUTED: Color = Color::Rgb(80, 80, 80); // Very dark gray
    const SUCCESS: Color = Color::Rgb(46, 204, 113); // Green
}

#[derive(Debug, Clone, PartialEq)]
enum AppMode {
    ProcessList,
    ConfirmKill,
    Search,
}

#[derive(Debug, Clone, PartialEq)]
enum SortBy {
    Port,
    Pid,
    User,
    Command,
    Memory,
    StartTime,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new().run(terminal);
    ratatui::restore();
    result
}

#[derive(Debug)]
pub struct App {
    running: bool,
    processes: Vec<lsof::LsofEntry>,
    filtered_processes: Vec<lsof::LsofEntry>,
    error_message: Option<String>,
    status_message: Option<String>,
    mode: AppMode,
    selected_index: usize,
    list_state: ListState,
    confirm_button_selected: bool,
    search_query: String,
    sort_by: SortBy,
    sort_ascending: bool,
    loading_animation_frame: usize,
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
            sort_ascending: false, // Default to descending for better UX
            loading_animation_frame: 0,
        }
    }
}

impl App {
    pub fn new() -> Self {
        let mut app = Self::default();
        app.status_message = Some("Loading processes...".to_string());
        app
    }

    pub fn refresh_processes(&mut self) {
        match lsof::get_listening_processes() {
            Ok(processes) => {
                self.processes = processes;
                self.apply_filter_and_sort();
                
                // If we have an active search but no filtered results, clear the search
                if !self.search_query.is_empty() && self.filtered_processes.is_empty() {
                    self.search_query.clear();
                    self.apply_filter_and_sort();
                }
                
                self.error_message = None;
                self.status_message = None;
                if self.selected_index >= self.filtered_processes.len() {
                    self.selected_index = 0;
                }
                self.list_state.select(if self.filtered_processes.is_empty() {
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

    fn apply_filter_and_sort(&mut self) {
        // Apply search filter
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

        // Apply sorting
        self.filtered_processes.sort_by(|a, b| {
            let comparison = match self.sort_by {
                SortBy::Port => {
                    let port_a = extract_port(&a.name);
                    let port_b = extract_port(&b.name);
                    port_a.cmp(&port_b)
                }
                SortBy::Pid => a.pid.parse::<u32>().unwrap_or(0).cmp(&b.pid.parse::<u32>().unwrap_or(0)),
                SortBy::User => a.user.cmp(&b.user),
                SortBy::Command => a.command.cmp(&b.command),
                SortBy::Memory => a.memory_mb.partial_cmp(&b.memory_mb).unwrap_or(std::cmp::Ordering::Equal),
                SortBy::StartTime => {
                    match (&a.start_time, &b.start_time) {
                        (Some(a_time), Some(b_time)) => a_time.cmp(b_time),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                }
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
                .min(animation_interval.checked_sub(last_animation.elapsed()).unwrap_or(Duration::from_secs(0)));
                
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

    fn render(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(5), Constraint::Min(0)])
            .split(frame.area());

        self.render_header(frame, chunks[0]);

        if let Some(error) = &self.error_message {
            let text = format!("Error: {}\n\nPress 'r' to retry, 'q' to quit.", error);
            frame.render_widget(
                Paragraph::new(text).block(Block::bordered()).centered(),
                chunks[1],
            );
            return;
        }

        if self.processes.is_empty() && self.status_message.is_some() {
            let text = self.status_message.as_ref().unwrap();
            let loading_spinner = get_loading_animation(self.loading_animation_frame);
            frame.render_widget(
                Paragraph::new(format!("{} {}\n\nPlease wait...", loading_spinner, text))
                    .style(Style::default().fg(Colors::TEXT_SECONDARY))
                    .centered(),
                chunks[1],
            );
            return;
        }

        // Show message when no processes are running
        if self.processes.is_empty() {
            let text = "🌿 No processes are currently listening on any ports\n\nEverything is quiet and peaceful!\n\nPress 'r' to refresh or 'q' to quit.";
            frame.render_widget(
                Paragraph::new(text)
                    .style(Style::default().fg(Colors::TEXT_SECONDARY))
                    .alignment(Alignment::Center)
                    .centered(),
                chunks[1],
            );
            return;
        }

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(4)])
            .split(chunks[1]);

        let list_items: Vec<ListItem> = self
            .filtered_processes
            .iter()
            .enumerate()
            .map(|(idx, process)| {
                let port = if let Some(port_part) = process.name.split(':').last() {
                    port_part.replace("(LISTEN)", "").trim().to_string()
                } else {
                    process.name.clone()
                };

                // Enhanced information display
                let protocol = process.get_protocol();
                let memory = process.get_memory_display();
                let uptime = process.get_relative_time();

                // Sophisticated selection indicator
                let is_selected = self.selected_index == idx;
                let (base_title_style, base_details_style, base_meta_style) = if is_selected {
                    (
                        Style::default().fg(Colors::ACCENT).bold(),
                        Style::default().fg(Colors::TEXT_PRIMARY),
                        Style::default().fg(Colors::TEXT_TERTIARY),
                    )
                } else {
                    (
                        Style::default().fg(Colors::TEXT_PRIMARY),
                        Style::default().fg(Colors::TEXT_SECONDARY),
                        Style::default().fg(Colors::TEXT_MUTED),
                    )
                };

                // Highlight style for the sorted field
                let sort_highlight_style = match self.sort_by {
                    SortBy::Port => Style::default().fg(Color::Rgb(241, 196, 15)).bold(), // Yellow
                    SortBy::Pid => Style::default().fg(Color::Rgb(52, 152, 219)).bold(),  // Blue
                    SortBy::User => Style::default().fg(Color::Rgb(46, 204, 113)).bold(), // Green
                    SortBy::Command => Style::default().fg(Color::Rgb(155, 89, 182)).bold(), // Purple
                    SortBy::Memory => Style::default().fg(Color::Rgb(231, 76, 60)).bold(), // Red
                    SortBy::StartTime => Style::default().fg(Color::Rgb(230, 126, 34)).bold(), // Orange
                };

                // Create lines with highlighted sort values
                let title_line = match self.sort_by {
                    SortBy::Port => {
                        ratatui::text::Line::from(vec![
                            ratatui::text::Span::styled(":", base_title_style),
                            ratatui::text::Span::styled(port.clone(), sort_highlight_style),
                            ratatui::text::Span::styled(format!(" • {} • {}", protocol, process.pid), base_title_style),
                        ])
                    },
                    SortBy::Pid => {
                        ratatui::text::Line::from(vec![
                            ratatui::text::Span::styled(format!(":{} • {} • ", port, protocol), base_title_style),
                            ratatui::text::Span::styled(process.pid.clone(), sort_highlight_style),
                        ])
                    },
                    _ => ratatui::text::Line::from(format!(":{} • {} • {}", port, protocol, process.pid)).style(base_title_style),
                };

                let details_line = match self.sort_by {
                    SortBy::User => {
                        ratatui::text::Line::from(vec![
                            ratatui::text::Span::styled("↳ ", base_details_style),
                            ratatui::text::Span::styled(process.user.clone(), sort_highlight_style),
                            ratatui::text::Span::styled(format!(" • {} • {}", process.command, memory), base_details_style),
                        ])
                    },
                    SortBy::Command => {
                        ratatui::text::Line::from(vec![
                            ratatui::text::Span::styled(format!("↳ {} • ", process.user), base_details_style),
                            ratatui::text::Span::styled(process.command.clone(), sort_highlight_style),
                            ratatui::text::Span::styled(format!(" • {}", memory), base_details_style),
                        ])
                    },
                    SortBy::Memory => {
                        ratatui::text::Line::from(vec![
                            ratatui::text::Span::styled(format!("↳ {} • {} • ", process.user, process.command), base_details_style),
                            ratatui::text::Span::styled(memory.clone(), sort_highlight_style),
                        ])
                    },
                    _ => ratatui::text::Line::from(format!("↳ {} • {} • {}", process.user, process.command, memory)).style(base_details_style),
                };

                let meta_line = match self.sort_by {
                    SortBy::StartTime => {
                        ratatui::text::Line::from(vec![
                            ratatui::text::Span::styled("  └ uptime: ", base_meta_style),
                            ratatui::text::Span::styled(format!("{} ago", uptime), sort_highlight_style),
                        ])
                    },
                    _ => ratatui::text::Line::from(format!("  └ uptime: {} ago", uptime)).style(base_meta_style),
                };

                ListItem::new(vec![
                    title_line,
                    details_line,
                    meta_line,
                    ratatui::text::Line::from(""),
                ])
            })
            .collect();

        let highlight_symbol = if self.mode == AppMode::Search {
            "🔍 "
        } else {
            "▶ "
        };

        let list = List::new(list_items)
            .highlight_style(
                Style::default()
                    .fg(Colors::ACCENT)
            )
            .highlight_symbol(highlight_symbol);

        frame.render_stateful_widget(list, main_chunks[0], &mut self.list_state);

        self.render_status_and_help(frame, main_chunks[1]);

        if self.mode == AppMode::ConfirmKill {
            self.render_confirmation_dialog(frame);
        }
    }

    fn render_header(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let title_text = "💀 Reaper";
        let desc_text = "A tiny program for viewing + killing ports";
        let process_count = self.filtered_processes.len();
        let total_count = self.processes.len();
        
        let info_text = if process_count == 0 && total_count == 0 {
            "Here's what's running...".to_string()
        } else if process_count != total_count {
            format!("{}/{} process{} (filtered by: \"{}\")", 
                process_count, total_count, 
                if total_count == 1 { "" } else { "es" },
                self.search_query)
        } else {
            format!("{} process{}", process_count, if process_count == 1 { "" } else { "es" })
        };

        let sort_text = format!("sorted by {} {} {}", 
            match self.sort_by {
                SortBy::Port => "port",
                SortBy::Pid => "pid",
                SortBy::User => "user", 
                SortBy::Command => "command",
                SortBy::Memory => "memory",
                SortBy::StartTime => "start time",
            },
            if self.sort_ascending { "↑" } else { "↓" },
            match self.sort_by {
                SortBy::Port => "🟡",
                SortBy::Pid => "🔵", 
                SortBy::User => "🟢",
                SortBy::Command => "🟣",
                SortBy::Memory => "🔴",
                SortBy::StartTime => "🟠",
            }
        );

        let header_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(
            Paragraph::new(title_text)
                .style(Style::default().fg(Colors::ACCENT).bold())
                .alignment(Alignment::Left),
            header_layout[0],
        );

        frame.render_widget(
            Paragraph::new(desc_text)
                .style(Style::default().fg(Colors::TEXT_SECONDARY))
                .alignment(Alignment::Left),
            header_layout[1],
        );

        frame.render_widget(
            Paragraph::new(info_text)
                .style(Style::default().fg(Colors::TEXT_TERTIARY))
                .alignment(Alignment::Left),
            header_layout[2],
        );

        frame.render_widget(
            Paragraph::new(sort_text)
                .style(Style::default().fg(Colors::TEXT_MUTED))
                .alignment(Alignment::Left),
            header_layout[3],
        );
    }

    fn render_status_and_help(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let help_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Status message
                Constraint::Length(1), // Help text
                Constraint::Length(1), // Border space
            ])
            .split(area);

        // Status message
        if let Some(status) = &self.status_message {
            frame.render_widget(
                Paragraph::new(format!("✓ {}", status)).style(Style::default().fg(Colors::SUCCESS)),
                help_layout[0],
            );
        }

        // Help text
        let help_text = match self.mode {
            AppMode::ProcessList => {
                if self.search_query.is_empty() {
                    "↑/↓: Navigate • Enter: Select • /: Search • s: Sort • r: Refresh • q/Esc: Quit"
                } else {
                    &format!("Search: \"{}\" • Esc: Clear search • ↑/↓: Navigate • Enter: Select", self.search_query)
                }
            }
            AppMode::ConfirmKill => "←/→: Select button • Enter: Confirm • y: Yes • n/Esc: No",
            AppMode::Search => "Type to search • Enter: Apply • Esc: Cancel",
        };

        frame.render_widget(
            Paragraph::new(help_text)
                .style(Style::default().fg(Colors::TEXT_MUTED))
                .alignment(Alignment::Center),
            help_layout[1],
        );
    }

    fn render_confirmation_dialog(&self, frame: &mut Frame) {
        if let Some(selected_process) = self.filtered_processes.get(self.selected_index) {
            let area = frame.area();

            let popup_area = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(30),
                    Constraint::Length(7),
                    Constraint::Percentage(63),
                ])
                .split(area)[1];

            let popup_area = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(20),
                    Constraint::Percentage(60),
                    Constraint::Percentage(20),
                ])
                .split(popup_area)[1];

            frame.render_widget(Clear, popup_area);

            let port = if let Some(port_part) = selected_process.name.split(':').last() {
                port_part.replace("(LISTEN)", "").trim().to_string()
            } else {
                selected_process.name.clone()
            };

            let question_text = format!("Are you sure you want to kill port :{}?", port);

            let dialog_content = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2), // Question text
                    Constraint::Length(1), // Spacing
                    Constraint::Length(3), // Buttons
                ])
                .split(popup_area);

            frame.render_widget(
                Paragraph::new(question_text)
                    .style(Style::default().fg(Colors::TEXT_PRIMARY))
                    .alignment(Alignment::Center)
                    .wrap(ratatui::widgets::Wrap { trim: true }),
                dialog_content[0],
            );

            let buttons_area = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(dialog_content[2]);

            let yes_style = if self.confirm_button_selected {
                Style::default()
                    .fg(Colors::ACCENT)
                    .bold()
            } else {
                Style::default()
                    .fg(Colors::TEXT_SECONDARY)
            };

            let yes_text = if self.confirm_button_selected {
                "► Yes ◄"
            } else {
                "Yes"
            };

            frame.render_widget(
                Paragraph::new(yes_text)
                    .style(yes_style)
                    .alignment(Alignment::Center),
                buttons_area[0],
            );

            let no_style = if !self.confirm_button_selected {
                Style::default()
                    .fg(Colors::ACCENT)
                    .bold()
            } else {
                Style::default()
                    .fg(Colors::TEXT_SECONDARY)
            };

            let no_text = if !self.confirm_button_selected {
                "► No, take me back ◄"
            } else {
                "No, take me back"
            };

            frame.render_widget(
                Paragraph::new(no_text)
                    .style(no_style)
                    .alignment(Alignment::Center),
                buttons_area[1],
            );
        }
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
                        // Clear search if there's an active search
                        self.search_query.clear();
                        self.apply_filter_and_sort();
                        self.selected_index = 0;
                        self.list_state.select(if self.filtered_processes.is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                    } else {
                        // Only quit if no search is active
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
                    self.list_state.select(if self.filtered_processes.is_empty() {
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
                    self.list_state.select(if self.filtered_processes.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                }
                (_, KeyCode::Char(c)) => {
                    self.search_query.push(c);
                    self.apply_filter_and_sort();
                    self.selected_index = 0;
                    self.list_state.select(if self.filtered_processes.is_empty() {
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
        self.list_state.select(if self.filtered_processes.is_empty() {
            None
        } else {
            Some(0)
        });
    }

    fn apply_search(&mut self) {
        self.mode = AppMode::ProcessList;
        self.apply_filter_and_sort();
        self.selected_index = 0;
        self.list_state.select(if self.filtered_processes.is_empty() {
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
            self.sort_ascending = false; // Default to descending for new sort
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
