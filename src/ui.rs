use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};

use crate::app::{App, AppMode, SortBy};
use crate::lsof::LsofEntry;

pub(crate) struct Colors;
impl Colors {
    pub(crate) const ACCENT: Color = Color::Rgb(26, 188, 156);
    pub(crate) const TEXT_PRIMARY: Color = Color::Rgb(240, 240, 240);
    pub(crate) const TEXT_SECONDARY: Color = Color::Rgb(180, 180, 180);
    pub(crate) const TEXT_TERTIARY: Color = Color::Rgb(120, 120, 120);
    pub(crate) const TEXT_MUTED: Color = Color::Rgb(80, 80, 80);
    pub(crate) const SUCCESS: Color = Color::Rgb(46, 204, 113);
}

fn get_loading_animation(frame: usize) -> &'static str {
    let animations = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    animations[frame % animations.len()]
}

fn highlight_matching_text(text: &str, query: &str, style: Style) -> Vec<Span<'static>> {
    if query.is_empty() {
        return vec![Span::styled(text.to_string(), style)];
    }

    let mut spans = Vec::new();
    let lower_text = text.to_lowercase();
    let lower_query = query.to_lowercase();

    let mut last_end = 0;
    for (start, _) in lower_text.match_indices(&lower_query) {
        if start > last_end {
            spans.push(Span::styled(text[last_end..start].to_string(), style));
        }

        let end = start + lower_query.len();
        spans.push(Span::styled(
            text[start..end].to_string(),
            style.add_modifier(ratatui::style::Modifier::UNDERLINED),
        ));

        last_end = end;
    }

    if last_end < text.len() {
        spans.push(Span::styled(text[last_end..].to_string(), style));
    }

    spans
}

fn sort_color(sort_by: &SortBy) -> Color {
    match sort_by {
        SortBy::Port => Color::Rgb(241, 196, 15),
        SortBy::Pid => Color::Rgb(52, 152, 219),
        SortBy::User => Color::Rgb(46, 204, 113),
        SortBy::Command => Color::Rgb(155, 89, 182),
        SortBy::Memory => Color::Rgb(231, 76, 60),
        SortBy::StartTime => Color::Rgb(230, 126, 34),
    }
}

fn sort_label(sort_by: &SortBy) -> &'static str {
    match sort_by {
        SortBy::Port => "port",
        SortBy::Pid => "pid",
        SortBy::User => "user",
        SortBy::Command => "command",
        SortBy::Memory => "memory",
        SortBy::StartTime => "start time",
    }
}

fn sort_marker(sort_by: &SortBy) -> &'static str {
    match sort_by {
        SortBy::Port => "🟡",
        SortBy::Pid => "🔵",
        SortBy::User => "🟢",
        SortBy::Command => "🟣",
        SortBy::Memory => "🔴",
        SortBy::StartTime => "🟠",
    }
}

impl App {
    pub(crate) fn render(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Min(0)])
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

        if let Some(text) = &self.loading_message
            && self.processes.is_empty()
        {
            let spinner = get_loading_animation(self.loading_animation_frame);
            frame.render_widget(
                Paragraph::new(format!("{} {}\n\nPlease wait...", spinner, text))
                    .style(Style::default().fg(Colors::TEXT_SECONDARY))
                    .centered(),
                chunks[1],
            );
            return;
        }

        if self.processes.is_empty() {
            let text = "🌿 All quiet on the network front!\n\nNo active processes are listening on any ports.\n\nPress 'r' to refresh or 'q' to quit.";
            frame.render_widget(
                Paragraph::new(text)
                    .style(Style::default().fg(Colors::TEXT_SECONDARY))
                    .alignment(Alignment::Center)
                    .centered(),
                chunks[1],
            );
            return;
        }

        if !self.search_query.is_empty() && self.filtered_processes.is_empty() {
            let text = format!(
                "🔍 Nothing found for \"{}\" - Try a different search term or press Esc to clear the search.",
                self.search_query
            );
            frame.render_widget(
                Paragraph::new(text)
                    .style(Style::default().fg(Colors::TEXT_SECONDARY))
                    .alignment(Alignment::Left),
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
            .map(|(idx, p)| self.build_list_item(p, idx))
            .collect();

        let highlight_symbol = if self.mode == AppMode::Search {
            "🔍 "
        } else {
            "▶ "
        };

        let list = List::new(list_items)
            .highlight_style(Style::default().fg(Colors::ACCENT))
            .highlight_symbol(highlight_symbol);

        frame.render_stateful_widget(list, main_chunks[0], &mut self.list_state);

        self.render_status_and_help(frame, main_chunks[1]);

        if self.mode == AppMode::ConfirmKill {
            self.render_confirmation_dialog(frame);
        }
    }

    fn build_list_item(&self, process: &LsofEntry, idx: usize) -> ListItem<'static> {
        let port = process.port.to_string();
        let protocol = process.get_protocol().to_string();
        let memory = process.get_memory_display();
        let uptime = process.get_relative_time();
        let is_selected = self.selected_index == idx;

        let (title_base, details_base, meta_base) = if is_selected {
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

        let sort_style = Style::default().fg(sort_color(&self.sort_by)).bold();

        // Either highlight search matches per-field, or highlight the active sort
        // column. Helper closure picks the right style for a given field.
        let field = |val: &str, this: SortBy| -> Vec<Span<'static>> {
            if !self.search_query.is_empty() {
                let style = if self.sort_by == this {
                    sort_style
                } else {
                    title_base
                };
                highlight_matching_text(val, &self.search_query, style)
            } else if self.sort_by == this {
                vec![Span::styled(val.to_string(), sort_style)]
            } else {
                vec![Span::styled(val.to_string(), title_base)]
            }
        };

        let detail_field = |val: &str, this: SortBy| -> Vec<Span<'static>> {
            if !self.search_query.is_empty() {
                let style = if self.sort_by == this {
                    sort_style
                } else {
                    details_base
                };
                highlight_matching_text(val, &self.search_query, style)
            } else if self.sort_by == this {
                vec![Span::styled(val.to_string(), sort_style)]
            } else {
                vec![Span::styled(val.to_string(), details_base)]
            }
        };

        let mut title = vec![Span::styled(":", title_base)];
        title.extend(field(&port, SortBy::Port));
        title.push(Span::styled(" • ", title_base));
        title.extend(field(&process.command, SortBy::Command));
        title.push(Span::styled(" • ", title_base));
        title.extend(field(&memory, SortBy::Memory));

        let mut details = vec![Span::styled("↳ ", details_base)];
        details.extend(detail_field(&process.user, SortBy::User));
        details.push(Span::styled(" • ", details_base));
        details.extend(highlight_matching_text(
            &protocol,
            &self.search_query,
            details_base,
        ));
        details.push(Span::styled(" • ", details_base));
        details.extend(detail_field(&process.pid, SortBy::Pid));

        let meta = if self.sort_by == SortBy::StartTime {
            Line::from(vec![
                Span::styled("  └ uptime: ", meta_base),
                Span::styled(format!("{} ago", uptime), sort_style),
            ])
        } else {
            Line::from(format!("  └ uptime: {} ago", uptime)).style(meta_base)
        };

        ListItem::new(vec![Line::from(title), Line::from(details), meta, Line::from("")])
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let title_text = "💀 Reaper";
        let desc_text = "A simple port management & process monitoring";
        let process_count = self.filtered_processes.len();
        let total_count = self.processes.len();

        let info_text = if process_count == 0 && total_count == 0 {
            "Scanning active ports...".to_string()
        } else if process_count != total_count {
            format!(
                "{}/{} process{} ",
                process_count,
                total_count,
                if total_count == 1 { "" } else { "es" }
            )
        } else {
            format!(
                "{} process{}",
                process_count,
                if process_count == 1 { "" } else { "es" }
            )
        };

        let info_widget = if self.mode == AppMode::Search {
            Paragraph::new(vec![ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(info_text, Style::default().fg(Colors::TEXT_TERTIARY)),
                ratatui::text::Span::styled(
                    " [searching: ",
                    Style::default().fg(Colors::TEXT_TERTIARY),
                ),
                ratatui::text::Span::styled(
                    if self.search_query.is_empty() {
                        "_"
                    } else {
                        &self.search_query
                    },
                    Style::default().fg(Colors::ACCENT).bold(),
                ),
                ratatui::text::Span::styled("]", Style::default().fg(Colors::TEXT_TERTIARY)),
            ])])
        } else if !self.search_query.is_empty() {
            Paragraph::new(vec![ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(info_text, Style::default().fg(Colors::TEXT_TERTIARY)),
                ratatui::text::Span::styled(
                    "(filtered by: \"",
                    Style::default().fg(Colors::TEXT_TERTIARY),
                ),
                ratatui::text::Span::styled(
                    self.search_query.clone(),
                    Style::default().fg(Colors::ACCENT).bold(),
                ),
                ratatui::text::Span::styled("\")", Style::default().fg(Colors::TEXT_TERTIARY)),
            ])])
        } else {
            Paragraph::new(info_text).style(Style::default().fg(Colors::TEXT_TERTIARY))
        };

        let sort_text = format!(
            "sorted by {} {} {}",
            sort_label(&self.sort_by),
            if self.sort_ascending { "↑" } else { "↓" },
            sort_marker(&self.sort_by)
        );

        let header_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(
            Paragraph::new(vec![ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(title_text, Style::default().fg(Colors::ACCENT).bold()),
                ratatui::text::Span::styled(" • ", Style::default().fg(Colors::TEXT_TERTIARY)),
                ratatui::text::Span::styled(
                    desc_text,
                    Style::default().fg(Colors::TEXT_SECONDARY).bold(),
                ),
            ])])
            .alignment(Alignment::Left),
            header_layout[0],
        );

        frame.render_widget(Paragraph::new(""), header_layout[1]);

        frame.render_widget(info_widget.alignment(Alignment::Left), header_layout[2]);

        frame.render_widget(
            Paragraph::new(sort_text)
                .style(Style::default().fg(Colors::TEXT_MUTED))
                .alignment(Alignment::Left),
            header_layout[3],
        );
    }

    fn render_status_and_help(&self, frame: &mut Frame, area: Rect) {
        let help_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        if let Some(status) = &self.status_message {
            frame.render_widget(
                Paragraph::new(format!("✓ {}", status)).style(Style::default().fg(Colors::SUCCESS)),
                help_layout[0],
            );
        }

        let help_text = match self.mode {
            AppMode::ProcessList => {
                if self.search_query.is_empty() {
                    "↑/↓: Navigate • Enter: Select • /: Search • s: Sort • r: Refresh • q/Esc: Quit"
                } else {
                    &format!(
                        "Search: \"{}\" • Esc: Clear search • ↑/↓: Navigate • Enter: Select",
                        self.search_query
                    )
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

            let question_text = format!(
                "Are you sure you want to kill port :{}?",
                selected_process.port
            );

            let dialog_content = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(3),
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
                Style::default().fg(Colors::ACCENT).bold()
            } else {
                Style::default().fg(Colors::TEXT_SECONDARY)
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
                Style::default().fg(Colors::ACCENT).bold()
            } else {
                Style::default().fg(Colors::TEXT_SECONDARY)
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
}
