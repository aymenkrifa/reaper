use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState},
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
    table_state: TableState,
}

impl Default for App {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            running: false,
            processes: Vec::new(),
            error_message: None,
            status_message: None,
            mode: AppMode::ProcessList,
            selected_index: 0,
            table_state,
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
                self.table_state.select(if self.processes.is_empty() {
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
        self.running = true;
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_crossterm_events()?;
        }
        Ok(())
    }

    fn render(&mut self, frame: &mut Frame) {
        let title = Line::from("Reaper - Process Monitor")
            .bold()
            .blue()
            .centered();

        if let Some(error) = &self.error_message {
            let text = format!("Error: {}\n\nPress 'r' to retry, 'q' to quit.", error);
            frame.render_widget(
                Paragraph::new(text)
                    .block(Block::bordered().title(title))
                    .centered(),
                frame.area(),
            );
            return;
        }

        let header = Row::new(vec![
            Cell::from("Command").bold(),
            Cell::from("PID").bold(),
            Cell::from("User").bold(),
            Cell::from("FD").bold(),
            Cell::from("Type").bold(),
            Cell::from("Device").bold(),
            Cell::from("Size/Off").bold(),
            Cell::from("Node").bold(),
            Cell::from("Name").bold(),
        ]);

        let rows: Vec<Row> = self
            .processes
            .iter()
            .map(|process| {
                Row::new(vec![
                    Cell::from(process.command.clone()),
                    Cell::from(process.pid.clone()),
                    Cell::from(process.user.clone()),
                    Cell::from(process.fd.clone()),
                    Cell::from(process.type_.clone()),
                    Cell::from(process.device.clone()),
                    Cell::from(process.size_off.clone()),
                    Cell::from(process.node.clone()),
                    Cell::from(process.name.clone()),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(15), // Command
                Constraint::Length(8),  // PID
                Constraint::Length(12), // User
                Constraint::Length(6),  // FD
                Constraint::Length(8),  // Type
                Constraint::Length(10), // Device
                Constraint::Length(10), // Size/Off
                Constraint::Length(8),  // Node
                Constraint::Min(20),    // Name (remaining space)
            ],
        )
        .header(header)
        .block(Block::bordered().title(title))
        .row_highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(">> ");

        frame.render_stateful_widget(table, frame.area(), &mut self.table_state);

        if let Some(status) = &self.status_message {
            let status_area = ratatui::layout::Rect {
                x: frame.area().x + 1,
                y: frame.area().bottom() - 3,
                width: frame.area().width - 2,
                height: 1,
            };

            frame.render_widget(
                Paragraph::new(format!("✓ {}", status)).style(Style::default().fg(Color::Green)),
                status_area,
            );
        }

        let help_area = ratatui::layout::Rect {
            x: frame.area().x + 1,
            y: frame.area().bottom() - 2,
            width: frame.area().width - 2,
            height: 1,
        };

        let help_text = match self.mode {
            AppMode::ProcessList => "↑/↓: Navigate, Enter: Select, r: Refresh, q/Esc/Ctrl-C: Quit",
            AppMode::ConfirmKill => "y: Confirm kill, n/Esc: Cancel",
        };

        frame.render_widget(
            Paragraph::new(help_text).style(Style::default().dim()),
            help_area,
        );

        if self.mode == AppMode::ConfirmKill {
            self.render_confirmation_dialog(frame);
        }
    }

    fn render_confirmation_dialog(&self, frame: &mut Frame) {
        if let Some(selected_process) = self.processes.get(self.selected_index) {
            let area = frame.area();

            let popup_area = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(25),
                    Constraint::Length(10),
                    Constraint::Percentage(65),
                ])
                .split(area)[1];

            let popup_area = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(25),
                    Constraint::Percentage(50),
                    Constraint::Percentage(25),
                ])
                .split(popup_area)[1];

            frame.render_widget(Clear, popup_area);

            let text = format!(
                "Kill Process?\n\nCommand: {}\nPID: {}\nUser: {}\nPort: {}\n\n[y] Yes  [n] No",
                selected_process.command,
                selected_process.pid,
                selected_process.user,
                selected_process.name
            );

            let dialog = Paragraph::new(text)
                .block(
                    Block::bordered()
                        .title("Confirm Action")
                        .style(Style::default().fg(Color::Yellow)),
                )
                .style(Style::default().bg(Color::DarkGray))
                .alignment(Alignment::Center);

            frame.render_widget(dialog, popup_area);
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
            self.table_state.select(Some(self.selected_index));
        }
    }

    fn select_next(&mut self) {
        if !self.processes.is_empty() {
            if self.selected_index < self.processes.len() - 1 {
                self.selected_index += 1;
            } else {
                self.selected_index = 0;
            }
            self.table_state.select(Some(self.selected_index));
        }
    }

    fn enter_confirm_mode(&mut self) {
        if !self.processes.is_empty() {
            self.mode = AppMode::ConfirmKill;
        }
    }

    fn confirm_kill(&mut self) {
        if let Some(process) = self.processes.get(self.selected_index) {
            let pid = &process.pid;
            let command = &process.command;

            // First try graceful kill (SIGTERM)
            match lsof::kill_process(pid) {
                Ok(()) => {
                    self.status_message =
                        Some(format!("Successfully killed process {} ({})", command, pid));
                    self.error_message = None;
                    self.mode = AppMode::ProcessList;

                    std::thread::sleep(std::time::Duration::from_millis(500));
                    self.refresh_processes();
                }
                Err(e) => {
                    match lsof::force_kill_process(pid) {
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
                    }
                }
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
