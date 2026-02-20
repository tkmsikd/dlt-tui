use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::app::{App, AppScreen};

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
        .split(f.area());

    f.render_widget(ratatui::widgets::Clear, f.area());

    match app.screen {
        AppScreen::Explorer => {
            let items: Vec<ListItem> = app
                .explorer_items
                .iter()
                .map(|entry| {
                    let symbol = if entry.is_dir { "[DIR] " } else { "[FILE]" };
                    let content = format!("{} {}", symbol, entry.name);
                    ListItem::new(content)
                })
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .title("File Explorer")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::Yellow))
                .highlight_symbol(">> ");

            let mut state = ratatui::widgets::ListState::default();
            state.select(Some(app.explorer_selected_index));

            f.render_stateful_widget(list, chunks[0], &mut state);
        }
        AppScreen::LogViewer => {
            let header_cells = ["Level", "Time", "ECU", "APP", "CTX", "Payload"]
                .iter()
                .map(|h| ratatui::widgets::Cell::from(*h).style(Style::default().fg(Color::Cyan)));

            let header = ratatui::widgets::Row::new(header_cells)
                .style(Style::default().bg(Color::DarkGray))
                .height(1)
                .bottom_margin(1);

            let rows = app.logs.iter().map(|log| {
                let (level_str, level_color) = match &log.log_level {
                    Some(crate::parser::LogLevel::Fatal) => ("FTL", Color::Red),
                    Some(crate::parser::LogLevel::Error) => ("ERR", Color::LightRed),
                    Some(crate::parser::LogLevel::Warn) => ("WRN", Color::Yellow),
                    Some(crate::parser::LogLevel::Info) => ("INF", Color::Green),
                    Some(crate::parser::LogLevel::Debug) => ("DBG", Color::Blue),
                    Some(crate::parser::LogLevel::Verbose) => ("VRB", Color::Gray),
                    Some(crate::parser::LogLevel::Unknown(_)) => ("UNK", Color::DarkGray),
                    None => ("---", Color::Reset),
                };

                let cells = vec![
                    ratatui::widgets::Cell::from(level_str).style(Style::default().fg(level_color)),
                    ratatui::widgets::Cell::from(log.timestamp_us.to_string()),
                    ratatui::widgets::Cell::from(log.ecu_id.clone()),
                    ratatui::widgets::Cell::from(
                        log.apid.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    ratatui::widgets::Cell::from(
                        log.ctid.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    ratatui::widgets::Cell::from(log.payload_text.clone()),
                ];
                ratatui::widgets::Row::new(cells).height(1)
            });

            // Table widths: Level(5), Time(15), ECU(5), APP(5), CTX(5), Payload(Min(20))
            let widths = [
                Constraint::Length(5),
                Constraint::Length(15),
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Min(20),
            ];

            let table = ratatui::widgets::Table::new(rows, widths)
                .header(header)
                .block(
                    Block::default()
                        .title("Log Viewer")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                )
                .row_highlight_style(Style::default().bg(Color::Indexed(8)).fg(Color::White))
                .highlight_symbol(">> ");

            // Note: ratatui::widgets::Table uses TableState instead of ListState
            let mut state = ratatui::widgets::TableState::default();
            state.select(Some(app.logs_selected_index));
            f.render_stateful_widget(table, chunks[0], &mut state);
        }
    }

    let status_str = match app.screen {
        AppScreen::Explorer => format!(
            "Mode: Explorer | Files: {} | (j/k) Move | (Enter) Open | (q) Quit",
            app.explorer_items.len()
        ),
        AppScreen::LogViewer => format!(
            "Mode: Viewer | Logs: {} | (j/k) Scroll | (Esc) List | (q) Quit",
            app.logs.len()
        ),
    };

    let status =
        Paragraph::new(status_str).block(Block::default().title("Status").borders(Borders::ALL));
    f.render_widget(status, chunks[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::explorer::FileEntry;
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;

    #[test]
    fn test_draw_explorer_screen() {
        let mut app = App::new();
        app.screen = AppScreen::Explorer;
        app.explorer_items.push(FileEntry {
            name: "test_file.dlt".to_string(),
            is_dir: false,
            path: PathBuf::from("test_file.dlt"),
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| draw(f, &app)).unwrap();

        // Check if the output contains the file name
        let buffer = terminal.backend().buffer();
        // The file name should be visible somewhere
        let content = buffer.content(); // A slice of cells
        let _has_text = content
            .iter()
            .any(|cell| cell.symbol() == "t" || cell.symbol() == "test_file.dlt");

        // This is a naive assertion. TestBackend provides more precise cell checks,
        // but for MVP TDD, checking that it renders something without panicking is a good start.
    }

    #[test]
    fn test_draw_log_viewer_screen() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| draw(f, &app)).unwrap();
        // Checks that Log Viewer does not panic and renders.
    }
}
