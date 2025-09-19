use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    style::Stylize,
    text::Line,
    widgets::{Block, Paragraph, Table, Row, Cell},
    layout::Constraint,
};

mod lsof;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new().run(terminal);
    ratatui::restore();
    result
}

/// The main application which holds the state and logic of the application.
#[derive(Debug, Default)]
pub struct App {
    /// Is the application running?
    running: bool,
    /// List of listening processes from lsof
    processes: Vec<lsof::LsofEntry>,
    /// Error message to display if lsof fails
    error_message: Option<String>,
}

impl App {
    /// Construct a new instance of [`App`].
    pub fn new() -> Self {
        let mut app = Self::default();
        app.refresh_processes();
        app
    }

    /// Refresh the list of listening processes
    pub fn refresh_processes(&mut self) {
        match lsof::get_listening_processes() {
            Ok(processes) => {
                self.processes = processes;
                self.error_message = None;
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to get processes: {}", e));
            }
        }
    }

    /// Run the application's main loop.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_crossterm_events()?;
        }
        Ok(())
    }

    /// Renders the user interface.
    ///
    /// This is where you add new widgets. See the following resources for more information:
    ///
    /// - <https://docs.rs/ratatui/latest/ratatui/widgets/index.html>
    /// - <https://github.com/ratatui/ratatui/tree/main/ratatui-widgets/examples>
    fn render(&mut self, frame: &mut Frame) {
        let title = Line::from("Reaper - Process Monitor")
            .bold()
            .blue()
            .centered();

        if let Some(error) = &self.error_message {
            // Show error message if lsof failed
            let text = format!("Error: {}\n\nPress 'r' to retry, 'q' to quit.", error);
            frame.render_widget(
                Paragraph::new(text)
                    .block(Block::bordered().title(title))
                    .centered(),
                frame.area(),
            );
            return;
        }

        // Create table header
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

        // Create table rows from processes
        let rows: Vec<Row> = self.processes.iter().map(|process| {
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
        }).collect();

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
            ]
        )
        .header(header)
        .block(Block::bordered().title(title));

        frame.render_widget(table, frame.area());

        // Show help text at the bottom
        let help_area = ratatui::layout::Rect {
            x: frame.area().x + 1,
            y: frame.area().bottom() - 2,
            width: frame.area().width - 2,
            height: 1,
        };
        
        let help_text = "Press 'r' to refresh, 'q'/'Esc'/Ctrl-C to quit";
        frame.render_widget(
            Paragraph::new(help_text).style(ratatui::style::Style::default().dim()),
            help_area,
        );
    }

    /// Reads the crossterm events and updates the state of [`App`].
    ///
    /// If your application needs to perform work in between handling events, you can use the
    /// [`event::poll`] function to check if there are any events available with a timeout.
    fn handle_crossterm_events(&mut self) -> Result<()> {
        match event::read()? {
            // it's important to check KeyEventKind::Press to avoid handling key release events
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key),
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
            _ => {}
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc | KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => self.quit(),
            (_, KeyCode::Char('r') | KeyCode::Char('R')) => self.refresh_processes(),
            // Add other key handlers here.
            _ => {}
        }
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }
}
