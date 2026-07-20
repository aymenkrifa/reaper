use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
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
    pub(crate) const WARNING: Color = Color::Rgb(230, 126, 34);
    pub(crate) const DANGER: Color = Color::Rgb(231, 76, 60);
    /// Background tint for the selected row — dark teal that complements
    /// ACCENT without flattening the per-cell foreground colors.
    pub(crate) const SELECTED_BG: Color = Color::Rgb(20, 60, 55);

    // Per-attribute hues. Reused both for the active sort-column
    // highlight and for status/confirmation messages so the user builds
    // a consistent visual association: yellow = port, purple = command,
    // blue = pid, etc.
    pub(crate) const PORT_HUE: Color = Color::Rgb(241, 196, 15);
    pub(crate) const PID_HUE: Color = Color::Rgb(52, 152, 219);
    pub(crate) const USER_HUE: Color = Color::Rgb(46, 204, 113);
    pub(crate) const COMMAND_HUE: Color = Color::Rgb(155, 89, 182);
    pub(crate) const MEMORY_HUE: Color = Color::Rgb(231, 76, 60);
    pub(crate) const STARTTIME_HUE: Color = Color::Rgb(230, 126, 34);
    pub(crate) const PROTOCOL_HUE: Color = Color::Rgb(0, 200, 220);
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
    // ASCII-only case folding: unlike `to_lowercase()`, it never changes
    // the byte length (e.g. 'İ' → "i̇"), so match offsets found in the
    // folded copy are guaranteed to be valid char boundaries in `text`.
    let lower_text = text.to_ascii_lowercase();
    let lower_query = query.to_ascii_lowercase();

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
        SortBy::Port => Colors::PORT_HUE,
        SortBy::Pid => Colors::PID_HUE,
        SortBy::User => Colors::USER_HUE,
        SortBy::Command => Colors::COMMAND_HUE,
        SortBy::Memory => Colors::MEMORY_HUE,
        SortBy::StartTime => Colors::STARTTIME_HUE,
        SortBy::Protocol => Colors::PROTOCOL_HUE,
    }
}

/// Replace the user's $HOME prefix with `~` so `/home/aymen/testing/x`
/// renders as `~/testing/x`. Only applies the shortening when the path
/// crosses a directory boundary, to avoid mangling unrelated paths that
/// happen to start with the same characters as $HOME.
fn shorten_path(path: &str) -> String {
    use std::sync::OnceLock;
    static HOME: OnceLock<String> = OnceLock::new();
    let home = HOME.get_or_init(|| std::env::var("HOME").unwrap_or_default());
    if home.is_empty() {
        return path.to_string();
    }
    if path == home {
        return "~".to_string();
    }
    if let Some(rest) = path.strip_prefix(home)
        && rest.starts_with('/')
    {
        return format!("~{}", rest);
    }
    path.to_string()
}

/// Truncate to `max` display chars, replacing the last char with `…` when
/// truncation occurs. Without this signal, a clipped cell looks identical
/// to one that fits and the user can't tell they're missing information.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

impl App {
    pub(crate) fn render(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(5), Constraint::Min(0)])
            .split(frame.area());

        self.render_header(frame, chunks[0]);

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
            .constraints([
                Constraint::Min(0),    // process table
                Constraint::Length(2), // detail line for the selected row
                Constraint::Length(4), // status + help
            ])
            .split(chunks[1]);

        let rows: Vec<Row> = self
            .filtered_processes
            .iter()
            .map(|p| self.build_row(p))
            .collect();

        let widths = [
            Constraint::Length(7),  // PORT
            Constraint::Length(14), // USER
            Constraint::Length(8),  // MEM
            Constraint::Length(8),  // UPTIME
            Constraint::Length(7),  // PROTO (room for "PROTO ↑" header — was 5)
            Constraint::Length(7),  // PID
            Constraint::Length(50), // COMMAND (last column, truncates if longer)
        ];

        let highlight_symbol = if self.mode == AppMode::Search {
            "🔍 "
        } else {
            "▶ "
        };

        let table = Table::new(rows, widths)
            .header(self.build_header_row())
            .row_highlight_style(Style::default().bg(Colors::SELECTED_BG).bold())
            .highlight_symbol(highlight_symbol)
            .column_spacing(2);

        frame.render_stateful_widget(table, main_chunks[0], &mut self.table_state);

        self.render_selected_detail(frame, main_chunks[1]);

        self.render_status_and_help(frame, main_chunks[2]);
    }

    /// Two-line panel directly below the table that shows the unabridged
    /// COMMAND and CWD for the currently-selected row. The table cells
    /// themselves truncate with `…` to stay scannable; this panel is where
    /// you read the full text.
    fn render_selected_detail(&self, frame: &mut Frame, area: Rect) {
        let Some(p) = self.filtered_processes.get(self.selected_index) else {
            return;
        };
        let cwd_display = p
            .cwd
            .as_deref()
            .map(shorten_path)
            .unwrap_or_else(|| "—".to_string());

        let lines = vec![
            Line::from(vec![
                Span::styled("▌ ", Style::default().fg(Colors::ACCENT).bold()),
                Span::styled(p.command.clone(), Style::default().fg(Colors::TEXT_PRIMARY)),
            ]),
            Line::from(vec![
                Span::styled("↳ ", Style::default().fg(Colors::TEXT_TERTIARY)),
                Span::styled(cwd_display, Style::default().fg(Colors::TEXT_SECONDARY)),
            ]),
        ];

        frame.render_widget(Paragraph::new(lines), area);
    }

    fn build_header_row(&self) -> Row<'static> {
        let base = Style::default().fg(Colors::TEXT_TERTIARY).bold();
        let active = Style::default().fg(sort_color(&self.sort_by)).bold();
        let arrow = if self.sort_ascending { "↑" } else { "↓" };

        let header_cell = |label: &'static str, this: SortBy| -> Cell<'static> {
            if self.sort_by == this {
                Cell::from(format!("{} {}", label, arrow)).style(active)
            } else {
                Cell::from(label).style(base)
            }
        };

        Row::new(vec![
            header_cell("PORT", SortBy::Port),
            header_cell("USER", SortBy::User),
            header_cell("MEM", SortBy::Memory),
            header_cell("UPTIME", SortBy::StartTime),
            header_cell("PROTO", SortBy::Protocol),
            header_cell("PID", SortBy::Pid),
            header_cell("COMMAND", SortBy::Command),
        ])
        .bottom_margin(1)
    }

    fn build_row(&self, p: &LsofEntry) -> Row<'static> {
        let base = Style::default().fg(Colors::TEXT_PRIMARY);
        let dim = Style::default().fg(Colors::TEXT_TERTIARY);
        let sort_style = Style::default().fg(sort_color(&self.sort_by)).bold();
        let killable = p.is_killable();

        // Per-cell styling: search match wins, then active sort column, then the
        // base/dim style depending on whether the row is actionable.
        let cell = |val: String, this: SortBy| -> Cell<'static> {
            let row_default = if killable { base } else { dim };
            let column_default = if self.sort_by == this {
                sort_style
            } else {
                row_default
            };
            if self.search_query.is_empty() {
                Cell::from(Line::from(Span::styled(val, column_default)))
            } else {
                Cell::from(Line::from(highlight_matching_text(
                    &val,
                    &self.search_query,
                    column_default,
                )))
            }
        };

        let uptime = if p.start_time.is_some() {
            p.get_relative_time()
        } else {
            "—".to_string()
        };
        let memory = if killable {
            p.get_memory_display()
        } else {
            "—".to_string()
        };
        // Cells whose content can exceed their column width get an explicit
        // ellipsis so a clipped cell is visually distinguishable from one
        // that fit. Narrow numeric/identifier columns aren't truncated —
        // they always fit their constraint.
        Row::new(vec![
            cell(format!(":{}", p.port), SortBy::Port),
            cell(truncate(&p.user, 14), SortBy::User),
            cell(memory, SortBy::Memory),
            cell(uptime, SortBy::StartTime),
            cell(p.protocol.to_string(), SortBy::Protocol),
            cell(p.pid.clone(), SortBy::Pid),
            cell(truncate(&p.command, 50), SortBy::Command),
        ])
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let title_text = "💀 Reaper";
        let desc_text = "A simple port management & process monitoring";
        let process_count = self.filtered_processes.len();
        let total_count = self.processes.len();
        let hidden = self.restricted_hidden_count();

        let info_text = if process_count == 0 && total_count == 0 {
            "Scanning active ports...".to_string()
        } else if process_count != total_count {
            let mut s = format!(
                "{}/{} process{} ",
                process_count,
                total_count,
                if total_count == 1 { "" } else { "es" }
            );
            if hidden > 0 {
                s.push_str(&format!("({} restricted hidden — press 'a') ", hidden));
            }
            s
        } else if hidden > 0 {
            format!(
                "{} process{} ({} restricted hidden — press 'a')",
                process_count,
                if process_count == 1 { "" } else { "es" },
                hidden
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
            Paragraph::new(vec![Line::from(vec![
                Span::styled(title_text, Style::default().fg(Colors::ACCENT).bold()),
                Span::styled(" • ", Style::default().fg(Colors::TEXT_TERTIARY)),
                Span::styled(
                    desc_text,
                    Style::default().fg(Colors::TEXT_SECONDARY).bold(),
                ),
            ])])
            .alignment(Alignment::Left),
            header_layout[0],
        );

        frame.render_widget(Paragraph::new(""), header_layout[1]);

        frame.render_widget(info_widget.alignment(Alignment::Left), header_layout[2]);
    }

    fn render_status_and_help(&self, frame: &mut Frame, area: Rect) {
        // ConfirmKill takes over the whole status/help band so the prompt
        // is unmissable; the table above still shows what's about to die.
        if self.mode == AppMode::ConfirmKill {
            self.render_kill_prompt(frame, area);
            return;
        }

        let help_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        if let Some(status) = &self.status_message {
            // The line carries its own ✓/✗ prefix and styling.
            frame.render_widget(Paragraph::new(status.clone()), help_layout[0]);
        }

        let help_text = match self.mode {
            AppMode::ProcessList => {
                if self.search_query.is_empty() {
                    "↑/↓: Navigate • Enter: Kill • /: Search • s: Sort • a: Show restricted • r: Refresh • q/Esc: Quit"
                } else {
                    &format!(
                        "Search: \"{}\" • Esc: Clear search • ↑/↓: Navigate • Enter: Kill",
                        self.search_query
                    )
                }
            }
            AppMode::ConfirmKill => unreachable!("handled above"),
            AppMode::Search => "Type to search • Enter: Apply • Esc: Cancel",
        };

        frame.render_widget(
            Paragraph::new(help_text)
                .style(Style::default().fg(Colors::TEXT_MUTED))
                .alignment(Alignment::Center),
            help_layout[1],
        );
    }

    /// Inline kill confirmation — replaces the status/help band in
    /// ConfirmKill mode. No popup, no clear-and-redraw, no button
    /// navigation: just the question (with port/command/pid colored to
    /// match their column hues) and a clear two-key choice underneath.
    fn render_kill_prompt(&self, frame: &mut Frame, area: Rect) {
        // Render from the snapshot taken when the prompt opened — the same
        // entry confirm_kill() will act on — never the live selection.
        let Some(p) = self.pending_kill.as_ref() else {
            return;
        };
        let dim = Style::default().fg(Colors::TEXT_TERTIARY);

        let prompt = Line::from(vec![
            Span::styled("Kill ", Style::default().fg(Colors::DANGER).bold()),
            Span::styled(
                format!(":{}", p.port),
                Style::default().fg(Colors::PORT_HUE).bold(),
            ),
            Span::styled("  ", dim),
            Span::styled(p.command.clone(), Style::default().fg(Colors::COMMAND_HUE)),
            Span::styled("  pid ", dim),
            Span::styled(p.pid.clone(), Style::default().fg(Colors::PID_HUE).bold()),
            Span::styled(" ?", Style::default().fg(Colors::TEXT_PRIMARY).bold()),
        ]);

        let choices = Line::from(vec![
            Span::styled("[y/Enter]", Style::default().fg(Colors::DANGER).bold()),
            Span::styled(" kill        ", Style::default().fg(Colors::TEXT_SECONDARY)),
            Span::styled("[n/Esc]", Style::default().fg(Colors::TEXT_TERTIARY).bold()),
            Span::styled(" cancel", Style::default().fg(Colors::TEXT_SECONDARY)),
        ]);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        frame.render_widget(Paragraph::new(prompt), layout[0]);
        frame.render_widget(Paragraph::new(""), layout[1]);
        frame.render_widget(Paragraph::new(choices), layout[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn highlight_survives_length_changing_lowercase() {
        // 'İ' (2 bytes) lowercases to "i̇" (3 bytes); with `to_lowercase()`
        // the match offsets shifted past the end of the original string
        // and slicing panicked.
        let spans = highlight_matching_text("İstanbul-app", "app", Style::default());
        assert_eq!(joined(&spans), "İstanbul-app");
    }

    #[test]
    fn highlight_marks_ascii_matches_case_insensitively() {
        let style = Style::default();
        let spans = highlight_matching_text("Nginx-nginx", "NGINX", style);
        assert_eq!(joined(&spans), "Nginx-nginx");
        let underlined: Vec<&str> = spans
            .iter()
            .filter(|s| {
                s.style
                    .add_modifier
                    .contains(ratatui::style::Modifier::UNDERLINED)
            })
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(underlined, vec!["Nginx", "nginx"]);
    }

    #[test]
    fn truncate_marks_clipped_cells() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("exactly-10", 10), "exactly-10");
        assert_eq!(truncate("very-long-command", 10), "very-long…");
        assert_eq!(truncate("anything", 0), "");
    }
}
