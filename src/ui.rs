use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::app::{App, AppScreen};

fn format_timestamp(us: u64) -> String {
    let total_secs = us / 1_000_000;
    let micros = us % 1_000_000;
    let hours = (total_secs / 3600) % 24;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    format!("{:02}:{:02}:{:02}.{:06}", hours, minutes, seconds, micros)
}

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

            let rows = app.filtered_log_indices.iter().map(|&idx| {
                let log = &app.logs[idx];
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
                    ratatui::widgets::Cell::from(format_timestamp(log.timestamp_us)),
                    ratatui::widgets::Cell::from(log.ecu_id.as_str()),
                    ratatui::widgets::Cell::from(
                        log.apid.as_deref().unwrap_or("-"),
                    ),
                    ratatui::widgets::Cell::from(
                        log.ctid.as_deref().unwrap_or("-"),
                    ),
                    ratatui::widgets::Cell::from(log.payload_text.as_str()),
                ];
                ratatui::widgets::Row::new(cells).height(1)
            });

            // Table widths: Level(5), Time(15), ECU(5), APP(5), CTX(5), Payload(Min(20))
            let widths = [
                Constraint::Length(5),
                Constraint::Length(21),
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
        AppScreen::LogDetail => {
            if let Some(&idx) = app.filtered_log_indices.get(app.logs_selected_index) {
                let log = &app.logs[idx];

                let detail_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
                    .split(chunks[0]);

                let meta_text = format!(
                    "Timestamp: {} ({} μs)\nECU ID: {}\nAPP ID: {}\nCTX ID: {}\nLevel: {:?}\n\nPayload Default Text: \n{}",
                    format_timestamp(log.timestamp_us),
                    log.timestamp_us,
                    log.ecu_id,
                    log.apid.as_deref().unwrap_or("-"),
                    log.ctid.as_deref().unwrap_or("-"),
                    log.log_level,
                    log.payload_text
                );

                let meta_para = Paragraph::new(meta_text).block(
                    Block::default()
                        .title("Log Metadata & Extracted Text")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
                f.render_widget(meta_para, detail_chunks[0]);

                let mut hex_lines = String::new();
                for chunk in log.payload_raw.chunks(16) {
                    let hex_parts: Vec<String> =
                        chunk.iter().map(|b| format!("{:02X}", b)).collect();
                    let char_parts: String = chunk
                        .iter()
                        .map(|&b| {
                            if (32..=126).contains(&b) {
                                b as char
                            } else {
                                '.'
                            }
                        })
                        .collect();

                    let mut hex_padded = hex_parts.join(" ");
                    while hex_padded.len() < 47 {
                        hex_padded.push(' ');
                    }

                    hex_lines.push_str(&format!("{}  |{}\n", hex_padded, char_parts));
                }

                let hex_para = Paragraph::new(hex_lines).block(
                    Block::default()
                        .title(format!(
                            "Payload Hex Dump ({} bytes)",
                            log.payload_raw.len()
                        ))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Magenta)),
                );
                f.render_widget(hex_para, detail_chunks[1]);
            }
        }
    }

    let (status_str, status_style) = if let Some(ref err) = app.error_message {
        (
            format!("ERROR: {} | [Press any key to dismiss]", err),
            Style::default().bg(Color::Red).fg(Color::White),
        )
    } else {
        let mut string = match app.screen {
            AppScreen::Explorer => format!(
                "Mode: Explorer | Files: {} | (j/k) Move | (^f/^b) Page | (Enter) Open | (q) Quit",
                app.explorer_items.len()
            ),
            AppScreen::LogViewer => {
                let mut actives = Vec::new();
                if let Some(ref t) = app.filter.text {
                    actives.push(format!("Text='{}'", t));
                }
                if let Some(ref t) = app.filter.app_id {
                    actives.push(format!("APP='{}'", t));
                }
                if let Some(ref t) = app.filter.ctx_id {
                    actives.push(format!("CTX='{}'", t));
                }
                if let Some(ref t) = app.filter.min_level {
                    actives.push(format!("Level={:?}", t));
                }
                let filter_str = if actives.is_empty() {
                    String::new()
                } else {
                    format!("Filters: [{}] | ", actives.join(", "))
                };

                let conn_str = if let Some(ref addr) = app.connection_info {
                    format!("[TCP: {}] ", addr)
                } else if app.is_loading {
                    "[LOADING...] ".to_string()
                } else {
                    String::new()
                };

                let tail_str = if app.auto_scroll { "[TAIL] " } else { "" };

                let recovered_str = if app.skipped_bytes > 0 {
                    format!("[RECOVERED: {} bytes skipped] ", app.skipped_bytes)
                } else {
                    String::new()
                };

                format!(
                    "Mode: Viewer | {}{}{}{}Logs: {}/{} | (^f/^b) Page | (/) Text | (l) Level | (a) APP | (c) CTX | (C) Clear",
                    conn_str,
                    tail_str,
                    recovered_str,
                    filter_str,
                    app.filtered_log_indices.len(),
                    app.logs.len()
                )
            }
            AppScreen::LogDetail => format!(
                "Mode: Detail | Log {}/{} | (j/k) Scroll Logs | (Esc) Back to Viewer",
                app.logs_selected_index + 1,
                app.filtered_log_indices.len()
            ),
        };

        if let Some(ref mode) = app.filter_input_mode {
            let prefix = match mode {
                crate::app::FilterInputMode::Text => "Search Text",
                crate::app::FilterInputMode::AppId => "Filter APP ID",
                crate::app::FilterInputMode::CtxId => "Filter CTX ID",
                crate::app::FilterInputMode::MinLevel => "Filter Min Level (F/E/W/I/D/V)",
            };
            string = format!("{}: {}_", prefix, app.filter_input);
        }

        (string, Style::default())
    };

    let status = Paragraph::new(status_str)
        .style(status_style)
        .block(Block::default().title("Status").borders(Borders::ALL));
    f.render_widget(status, chunks[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::explorer::FileEntry;
    use crate::parser::DltMessage;
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;

    /// Extract all text from a TestBackend buffer as a single string
    fn buffer_to_string(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                text.push_str(cell.symbol());
            }
        }
        text
    }

    #[test]
    fn test_draw_explorer_screen() {
        let mut app = App::new();
        app.screen = AppScreen::Explorer;
        app.explorer_items.push(FileEntry {
            name: "test_file.dlt".to_string(),
            is_dir: false,
            path: PathBuf::from("test_file.dlt"),
        });

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(text.contains("File Explorer"), "Should show 'File Explorer' title");
        assert!(text.contains("test_file.dlt"), "Should show the file name");
        assert!(text.contains("Mode: Explorer"), "Should show Explorer mode in status bar");
    }

    #[test]
    fn test_draw_log_viewer_screen() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        app.logs.push(DltMessage {
            timestamp_us: 1_640_995_200_000_000,
            ecu_id: "ECU1".to_string(),
            apid: Some("DIAG".to_string()),
            ctid: Some("CAN1".to_string()),
            log_level: Some(crate::parser::LogLevel::Error),
            payload_text: "CAN bus timeout".to_string(),
            payload_raw: b"CAN bus timeout".to_vec(),
        });
        app.apply_filter();

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(text.contains("Log Viewer"), "Should show 'Log Viewer' title");
        assert!(text.contains("Level"), "Should show column header");
        assert!(text.contains("Payload"), "Should show Payload column");
        assert!(text.contains("ECU1"), "Should show ECU ID");
        assert!(text.contains("DIAG"), "Should show APP ID");
        assert!(text.contains("CAN bus timeout"), "Should show payload text");
        assert!(text.contains("Logs: 1/1"), "Should show log count");
    }

    #[test]
    fn test_draw_log_detail_screen() {
        let mut app = App::new();
        app.screen = AppScreen::LogDetail;
        app.logs.push(DltMessage {
            timestamp_us: 5_000_000,
            ecu_id: "ECU2".to_string(),
            apid: Some("NAV".to_string()),
            ctid: Some("GPS1".to_string()),
            log_level: Some(crate::parser::LogLevel::Info),
            payload_text: "GPS fix acquired".to_string(),
            payload_raw: b"GPS fix acquired".to_vec(),
        });
        app.apply_filter();

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(text.contains("Log Metadata"), "Should show metadata section");
        assert!(text.contains("Hex Dump"), "Should show hex dump section");
        assert!(text.contains("ECU2"), "Should show ECU ID in detail");
        assert!(text.contains("GPS fix acquired"), "Should show payload text");
    }

    #[test]
    fn test_draw_error_message() {
        let mut app = App::new();
        app.screen = AppScreen::Explorer;
        app.error_message = Some("File not found".to_string());

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(text.contains("ERROR"), "Should show ERROR prefix");
        assert!(text.contains("File not found"), "Should show error message");
    }

    #[test]
    fn test_draw_filter_input_mode() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        app.filter_input_mode = Some(crate::app::FilterInputMode::Text);
        app.filter_input = "CAN".to_string();

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(text.contains("Search Text"), "Should show search prompt");
        assert!(text.contains("CAN"), "Should show current input text");
    }
}
