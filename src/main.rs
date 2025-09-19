use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph},
    DefaultTerminal, Frame,
};

mod lsof;

#[derive(Debug, Clone, PartialEq)]
enum AppMode {
    ProcessList,
    ConfirmKill,
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
    error_message: Option<String>,
    status_message: Option<String>,
    mode: AppMode,
    selected_index: usize,
    list_state: ListState,
    confirm_button_selected: bool,
}

impl Default for App {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Self {
            running: false,
            processes: Vec::new(),
            error_message: None,
            status_message: None,
            mode: AppMode::ProcessList,
            selected_index: 0,
            list_state,
            confirm_button_selected: true,
        }
    }
}

impl App {
    pub fn new() -> Self {
        let mut app = Self::default();
        app.refresh_processes();
        app
    }

    pub fn refresh_processes(&mut self) {
        match lsof::get_listening_processes() {
            Ok(processes) => {
                self.processes = processes;
                self.error_message = None;
                self.status_message = None;
                if self.selected_index >= self.processes.len() {
                    self.selected_index = 0;
                }
                self.list_state.select(if self.processes.is_empty() {
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

    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        use std::time::{Duration, Instant};
        self.running = true;
        let refresh_interval = Duration::from_secs(1);
        let mut last_refresh = Instant::now();
        while self.running {
            let timeout = refresh_interval
                .checked_sub(last_refresh.elapsed())
                .unwrap_or(Duration::from_secs(0));
            if crossterm::event::poll(timeout)? {
                self.handle_crossterm_events()?;
            }
            if last_refresh.elapsed() >= refresh_interval {
                self.refresh_processes();
                last_refresh = Instant::now();
            }
            terminal.draw(|frame| self.render(frame))?;
        }
        Ok(())
    }

    fn render(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Min(0)])
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

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(chunks[1]);

        // Create list items in gruyere style: "Port :8080 (1234)" as title, "User: john, Command: node" as description
        let list_items: Vec<ListItem> = self
            .processes
            .iter()
            .map(|process| {
                let port = if let Some(port_part) = process.name.split(':').last() {
                    port_part.replace("(LISTEN)", "").trim().to_string()
                } else {
                    process.name.clone()
                };

                let title = format!("Port :{} ({})", port, process.pid);
                let description = format!("User: {}, Command: {}", process.user, process.command);
                
                ListItem::new(vec![
                    ratatui::text::Line::from(title).style(Style::default().fg(Color::White)),
                    ratatui::text::Line::from(description).style(Style::default().fg(Color::Gray)),
                    ratatui::text::Line::from(""),
                ])
            })
            .collect();

        let list = List::new(list_items)
            .block(Block::bordered().border_style(Style::default().fg(Color::Rgb(135, 75, 253))))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(238, 111, 248))
                    .fg(Color::White),
            )
            .highlight_symbol(">> ");

        frame.render_stateful_widget(list, main_chunks[0], &mut self.list_state);

        self.render_status_and_help(frame, main_chunks[1]);

        if self.mode == AppMode::ConfirmKill {
            self.render_confirmation_dialog(frame);
        }
    }

    fn render_header(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let title_text = "💀 Reaper";
        let desc_text = "A tiny program for viewing + killing ports";
        let info_text = "Here's what's running...";

        let header_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(
            Paragraph::new(title_text)
                .style(Style::default().fg(Color::Rgb(238, 111, 248)).bold())
                .alignment(Alignment::Left),
            header_layout[0],
        );

        frame.render_widget(
            Paragraph::new(desc_text)
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Left),
            header_layout[1],
        );

        frame.render_widget(
            Paragraph::new(info_text)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Left),
            header_layout[2],
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
                Paragraph::new(format!("✓ {}", status)).style(Style::default().fg(Color::Green)),
                help_layout[0],
            );
        }

        // Help text
        let help_text = match self.mode {
            AppMode::ProcessList => "↑/↓: Navigate • Enter: Select • r: Refresh • q/Esc: Quit",
            AppMode::ConfirmKill => "←/→: Select button • Enter: Confirm • y: Yes • n/Esc: No",
        };

        frame.render_widget(
            Paragraph::new(help_text)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center),
            help_layout[1],
        );
    }

    fn render_confirmation_dialog(&self, frame: &mut Frame) {
        if let Some(selected_process) = self.processes.get(self.selected_index) {
            let area = frame.area();

            let popup_area = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(30),
                    Constraint::Length(9),
                    Constraint::Percentage(61),
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
                .margin(1)
                .constraints([
                    Constraint::Length(2), // Question text
                    Constraint::Length(1), // Spacing
                    Constraint::Length(3), // Buttons
                ])
                .split(popup_area);

            frame.render_widget(
                Paragraph::new(question_text)
                    .style(Style::default().fg(Color::White))
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
                    .fg(Color::Black)
                    .bg(Color::Rgb(238, 111, 248))
            } else {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(100, 40, 100))
            };

            let yes_border_style = if self.confirm_button_selected {
                Style::default().fg(Color::Yellow).bold()
            } else {
                Style::default().fg(Color::Gray)
            };

            let yes_text = if self.confirm_button_selected {
                "► Yes ◄"
            } else {
                "Yes"
            };

            frame.render_widget(
                Paragraph::new(yes_text)
                    .style(yes_style)
                    .alignment(Alignment::Center)
                    .block(Block::bordered().border_style(yes_border_style)),
                buttons_area[0],
            );

            let no_style = if !self.confirm_button_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(136, 139, 126))
            } else {
                Style::default().fg(Color::White).bg(Color::Rgb(60, 60, 60))
            };

            let no_border_style = if !self.confirm_button_selected {
                Style::default().fg(Color::Yellow).bold()
            } else {
                Style::default().fg(Color::Gray)
            };

            let no_text = if !self.confirm_button_selected {
                "► No, take me back ◄"
            } else {
                "No, take me back"
            };

            frame.render_widget(
                Paragraph::new(no_text)
                    .style(no_style)
                    .alignment(Alignment::Center)
                    .block(Block::bordered().border_style(no_border_style)),
                buttons_area[1],
            );

            frame.render_widget(
                Block::bordered()
                    .border_style(Style::default().fg(Color::Rgb(135, 75, 253)))
                    .title("Confirm Action")
                    .title_style(Style::default().fg(Color::Rgb(135, 75, 253))),
                popup_area,
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
                (_, KeyCode::Esc | KeyCode::Char('q'))
                | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => self.quit(),
                (_, KeyCode::Char('r') | KeyCode::Char('R')) => self.refresh_processes(),
                (_, KeyCode::Up) => self.select_previous(),
                (_, KeyCode::Down) => self.select_next(),
                (_, KeyCode::Enter) => self.enter_confirm_mode(),
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
        }
    }

    fn select_previous(&mut self) {
        if !self.processes.is_empty() {
            if self.selected_index > 0 {
                self.selected_index -= 1;
            } else {
                self.selected_index = self.processes.len() - 1;
            }
            self.list_state.select(Some(self.selected_index));
        }
    }

    fn select_next(&mut self) {
        if !self.processes.is_empty() {
            if self.selected_index < self.processes.len() - 1 {
                self.selected_index += 1;
            } else {
                self.selected_index = 0;
            }
            self.list_state.select(Some(self.selected_index));
        }
    }

    fn enter_confirm_mode(&mut self) {
        if !self.processes.is_empty() {
            self.mode = AppMode::ConfirmKill;
            self.confirm_button_selected = true;
        }
    }

    fn confirm_kill(&mut self) {
        if let Some(process) = self.processes.get(self.selected_index) {
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
