use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::ops::Range;

use crate::app::{App, AppScreen, SidebarFocus};

pub fn format_timestamp(us: u64) -> String {
    let total_secs = us / 1_000_000;
    let micros = us % 1_000_000;
    let hours = (total_secs / 3600) % 24;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    format!("{:02}:{:02}:{:02}.{:06}", hours, minutes, seconds, micros)
}

pub fn format_message_timestamp(message: &crate::parser::DltMessage) -> String {
    use crate::parser::TimestampDomain;

    match message.timestamp_domain {
        TimestampDomain::StorageAbsolute => format_timestamp(message.timestamp_us),
        TimestampDomain::DaemonRelative => format!("T+{} ticks", message.timestamp_us / 100),
        TimestampDomain::Missing => "N/A".to_string(),
        TimestampDomain::Unspecified if message.timestamp_us == 0 => "N/A".to_string(),
        TimestampDomain::Unspecified => format_timestamp(message.timestamp_us),
    }
}

fn format_delta_time(
    current: &crate::parser::DltMessage,
    previous: Option<&crate::parser::DltMessage>,
    same_source: bool,
) -> String {
    use crate::parser::TimestampDomain;

    let Some(previous) = previous else {
        return if matches!(current.timestamp_domain, TimestampDomain::Missing)
            || (current.timestamp_domain == TimestampDomain::Unspecified
                && current.timestamp_us == 0)
        {
            "N/A".to_string()
        } else {
            "+0.000000s".to_string()
        };
    };
    if current.timestamp_domain != previous.timestamp_domain {
        return "N/A".to_string();
    }

    let delta = match current.timestamp_domain {
        TimestampDomain::Missing => None,
        TimestampDomain::DaemonRelative if same_source => {
            if current.timestamp_us >= previous.timestamp_us {
                Some(current.timestamp_us - previous.timestamp_us)
            } else {
                const WRAP_PERIOD_US: u64 = (u32::MAX as u64 + 1) * 100;
                let backwards = previous.timestamp_us - current.timestamp_us;
                (backwards > WRAP_PERIOD_US / 2)
                    .then_some(WRAP_PERIOD_US - previous.timestamp_us + current.timestamp_us)
            }
        }
        TimestampDomain::DaemonRelative => None,
        TimestampDomain::StorageAbsolute | TimestampDomain::Unspecified => {
            Some(current.timestamp_us.abs_diff(previous.timestamp_us))
        }
    };
    match delta {
        Some(delta) => format!("+{}.{:06}s", delta / 1_000_000, delta % 1_000_000),
        None => "N/A".to_string(),
    }
}

fn ctx_color(ctx: &str) -> Color {
    const PALETTE: [Color; 12] = [
        Color::Cyan,
        Color::Green,
        Color::Yellow,
        Color::Magenta,
        Color::LightBlue,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightMagenta,
        Color::LightCyan,
        Color::Indexed(208),
        Color::Indexed(141),
        Color::Indexed(75),
    ];

    if ctx == "-" || ctx.is_empty() {
        return Color::Gray;
    }

    let hash = ctx.bytes().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as usize)
    });
    PALETTE[hash % PALETTE.len()]
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
            let viewer_chunks = if chunks[0].width >= 100 {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(30), Constraint::Min(40)].as_ref())
                    .split(chunks[0])
            } else {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(0), Constraint::Min(20)].as_ref())
                    .split(chunks[0])
            };

            if viewer_chunks[0].width > 0 {
                draw_context_sidebar(f, app, viewer_chunks[0]);
            }

            let time_header = if app.show_time_delta {
                "Delta Time"
            } else {
                "Time"
            };
            let cols = [
                "Level",
                time_header,
                "ECU",
                "APP",
                "CTX",
                "Source",
                "Payload",
            ];
            let header_cells = cols
                .iter()
                .map(|h| ratatui::widgets::Cell::from(*h).style(Style::default().fg(Color::Cyan)));

            let header = ratatui::widgets::Row::new(header_cells)
                .style(Style::default().bg(Color::DarkGray))
                .height(1)
                .bottom_margin(1);

            let row_capacity = viewer_chunks[1].height.saturating_sub(4).max(1) as usize;
            let row_range = visible_log_range(
                app.filtered_log_indices.len(),
                app.logs_selected_index,
                row_capacity,
            );
            let row_start = row_range.start;
            let visible_indices = &app.filtered_log_indices[row_range];
            let rows = visible_indices.iter().enumerate().map(|(visible_i, &idx)| {
                let i = row_start + visible_i;
                let entry = &app.logs[idx];
                let log = &entry.message;
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

                let payload_display = if app.horizontal_scroll > 0 {
                    let chars: String = log
                        .payload_text()
                        .chars()
                        .skip(app.horizontal_scroll)
                        .collect();
                    if chars.is_empty() {
                        " ".to_string()
                    } else {
                        chars
                    }
                } else {
                    log.payload_text().to_string()
                };

                let time_str = if app.show_time_delta {
                    let previous = i.checked_sub(1).map(|previous_i| {
                        let previous_idx = app.filtered_log_indices[previous_i];
                        let previous_entry = &app.logs[previous_idx];
                        (
                            &previous_entry.message,
                            previous_entry.source_index == entry.source_index,
                        )
                    });
                    format_delta_time(
                        log,
                        previous.as_ref().map(|(message, _)| *message),
                        previous.is_some_and(|(_, same_source)| same_source),
                    )
                } else {
                    format_message_timestamp(log)
                };

                let cells = vec![
                    ratatui::widgets::Cell::from(level_str).style(Style::default().fg(level_color)),
                    ratatui::widgets::Cell::from(time_str),
                    ratatui::widgets::Cell::from(log.ecu_id.as_str()),
                    ratatui::widgets::Cell::from(log.apid.as_deref().unwrap_or("-")),
                    ratatui::widgets::Cell::from(log.ctid.as_deref().unwrap_or("-"))
                        .style(Style::default().fg(ctx_color(log.ctid.as_deref().unwrap_or("-")))),
                    ratatui::widgets::Cell::from(entry.source_identity()),
                    ratatui::widgets::Cell::from(payload_display),
                ];
                ratatui::widgets::Row::new(cells).height(1)
            });

            // Table widths: Level, Time, ECU, APP, CTX, Source, Payload
            let widths = [
                Constraint::Length(5),
                Constraint::Length(21),
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Length(18),
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
            if !app.filtered_log_indices.is_empty() {
                state.select(Some(
                    app.logs_selected_index
                        .min(app.filtered_log_indices.len() - 1)
                        - row_start,
                ));
            }
            f.render_stateful_widget(table, viewer_chunks[1], &mut state);
        }
        AppScreen::LogDetail => {
            if let Some(&idx) = app.filtered_log_indices.get(app.logs_selected_index) {
                let entry = &app.logs[idx];
                let log = &entry.message;

                let detail_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
                    .split(chunks[0]);

                let meta_text = format!(
                    "Timestamp: {} ({} μs, {:?})\nECU ID: {}\nAPP ID: {}\nCTX ID: {}\nSource: {}\nLevel: {:?}\n\nPayload Default Text: \n{}",
                    format_message_timestamp(log),
                    log.timestamp_us,
                    log.timestamp_domain,
                    log.ecu_id,
                    log.apid.as_deref().unwrap_or("-"),
                    log.ctid.as_deref().unwrap_or("-"),
                    entry.source_identity(),
                    log.log_level,
                    log.payload_text()
                );

                let meta_para = Paragraph::new(meta_text).block(
                    Block::default()
                        .title("Log Metadata & Extracted Text")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
                f.render_widget(
                    meta_para.scroll((app.detail_scroll.min(u16::MAX as usize) as u16, 0)),
                    detail_chunks[0],
                );

                let payload = log.payload_raw();
                let total_rows = payload.len().div_ceil(16);
                let visible_rows = detail_chunks[1].height.saturating_sub(2) as usize;
                let start_row = app
                    .detail_scroll
                    .min(total_rows.saturating_sub(visible_rows));
                let end_row = (start_row + visible_rows).min(total_rows);
                let mut hex_lines = String::new();
                for row in start_row..end_row {
                    let start_byte = row * 16;
                    let chunk = &payload[start_byte..(start_byte + 16).min(payload.len())];
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

                    hex_lines.push_str(&format!(
                        "{:08X}  {}  |{}\n",
                        start_byte, hex_padded, char_parts
                    ));
                }

                let hex_para = Paragraph::new(hex_lines).block(
                    Block::default()
                        .title(format!(
                            "Payload Hex Dump ({} bytes, rows {}-{})",
                            payload.len(),
                            if end_row == 0 { 0 } else { start_row + 1 },
                            end_row
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
    } else if let Some(ref info) = app.info_message {
        (
            format!("INFO: {} | [Press any key to dismiss]", info),
            Style::default().bg(Color::Blue).fg(Color::White),
        )
    } else {
        let mut string = match app.screen {
            AppScreen::Explorer => format!(
                "Mode: Explorer | Files: {} | (j/k) Move | (^f/^b) Page | (Enter) Open | (b) Batch load | (q) Quit",
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

                // UX-2: shortened status bar to fit ~80 columns
                format!(
                    "Viewer | {}{}{}{}Logs: {}/{} | /text l=lvl a=app c=ctx C=clr S/L=cfg t=Δ E=exp",
                    conn_str,
                    tail_str,
                    recovered_str,
                    filter_str,
                    app.filtered_log_indices.len(),
                    app.logs.len()
                )
            }
            AppScreen::LogDetail => {
                if app.filtered_log_indices.is_empty() {
                    "Mode: Detail | No matching logs | (Esc) Back to Viewer".to_string()
                } else {
                    format!(
                        "Mode: Detail | Log {}/{} | (j/k) Scroll Logs | (Esc) Back to Viewer",
                        app.logs_selected_index + 1,
                        app.filtered_log_indices.len()
                    )
                }
            }
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

fn visible_log_range(len: usize, selected: usize, capacity: usize) -> Range<usize> {
    if len == 0 || capacity == 0 {
        return 0..0;
    }

    let selected = selected.min(len - 1);
    let start = selected.saturating_sub(capacity - 1);
    start..(start + capacity).min(len)
}

fn draw_context_sidebar(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let sidebar_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(area);
    let selected_source = app.selected_entry().map(|entry| entry.source_identity());
    let selected_ctx = app
        .selected_entry()
        .and_then(|entry| entry.message.ctid.clone());

    let source_capacity = sidebar_chunks[0].height.saturating_sub(2) as usize;
    let mut source_counts: Vec<_> = app.source_counts().iter().collect();
    source_counts.sort_unstable_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let source_range = visible_sidebar_range(
        source_counts.len(),
        app.source_sidebar_selected_index,
        source_capacity,
    );
    let source_items: Vec<_> = source_counts
        .iter()
        .enumerate()
        .skip(source_range.start)
        .take(source_range.len())
        .map(|(index, (source, count))| {
            let selected = if app.sidebar_focus == SidebarFocus::Sources {
                index == app.source_sidebar_selected_index
            } else {
                Some(source.as_str()) == selected_source.as_deref()
            };
            let marker = if selected { ">" } else { " " };
            ListItem::new(format!("{} {:>5} {}", marker, count, source))
        })
        .collect();

    let ctx_capacity = sidebar_chunks[1].height.saturating_sub(2) as usize;
    let ctx_counts = app.ranked_ctx_entries();
    let ctx_range = visible_sidebar_range(
        ctx_counts.len(),
        app.ctx_sidebar_selected_index,
        ctx_capacity,
    );
    let ctx_items: Vec<_> = ctx_counts
        .iter()
        .enumerate()
        .skip(ctx_range.start)
        .take(ctx_range.len())
        .map(|(index, (ctx, count))| {
            let label = ctx.as_deref().unwrap_or("-");
            let selected = if app.sidebar_focus == SidebarFocus::Contexts {
                index == app.ctx_sidebar_selected_index
            } else {
                ctx == &selected_ctx
            };
            let marker = if selected { ">" } else { " " };
            ListItem::new(format!("{} {:>5} {}", marker, count, label))
                .style(Style::default().fg(ctx_color(label)))
        })
        .collect();

    let sources = List::new(source_items).block(
        Block::default()
            .title("Sources")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    let mut source_state = ratatui::widgets::ListState::default();
    if app.sidebar_focus == SidebarFocus::Sources && source_capacity > 0 {
        source_state.select(Some(
            app.source_sidebar_selected_index
                .saturating_sub(source_range.start),
        ));
    }
    f.render_stateful_widget(sources, sidebar_chunks[0], &mut source_state);

    let contexts = List::new(ctx_items).block(
        Block::default()
            .title("Contexts")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta)),
    );
    let mut ctx_state = ratatui::widgets::ListState::default();
    if app.sidebar_focus == SidebarFocus::Contexts && ctx_capacity > 0 {
        ctx_state.select(Some(
            app.ctx_sidebar_selected_index
                .saturating_sub(ctx_range.start),
        ));
    }
    f.render_stateful_widget(contexts, sidebar_chunks[1], &mut ctx_state);
}

fn visible_sidebar_range(len: usize, selected: usize, capacity: usize) -> Range<usize> {
    if len == 0 || capacity == 0 {
        return 0..0;
    }
    let selected = selected.min(len - 1);
    let start = selected.saturating_sub(capacity - 1);
    start..(start + capacity).min(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::explorer::FileEntry;
    use crate::{app::LogEntry, parser::DltMessage};
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
        assert!(
            text.contains("File Explorer"),
            "Should show 'File Explorer' title"
        );
        assert!(text.contains("test_file.dlt"), "Should show the file name");
        assert!(
            text.contains("Mode: Explorer"),
            "Should show Explorer mode in status bar"
        );
    }

    #[test]
    fn test_draw_log_viewer_screen() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        app.logs.push(LogEntry::new(
            DltMessage::new(
                1_640_995_200_000_000,
                "ECU1".to_string(),
                Some("DIAG".to_string()),
                Some("CAN1".to_string()),
                Some(crate::parser::LogLevel::Error),
                b"CAN bus timeout".to_vec(),
            ),
            Some(PathBuf::from("diag_can1.dlt")),
            0,
        ));
        app.apply_filter();

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(
            text.contains("Log Viewer"),
            "Should show 'Log Viewer' title"
        );
        assert!(text.contains("Level"), "Should show column header");
        assert!(text.contains("Payload"), "Should show Payload column");
        assert!(text.contains("Sources"), "Should show source sidebar");
        assert!(text.contains("Contexts"), "Should show context sidebar");
        assert!(text.contains("diag_can1.dlt"), "Should show source file");
        assert!(text.contains("ECU1"), "Should show ECU ID");
        assert!(text.contains("DIAG"), "Should show APP ID");
        assert!(text.contains("CAN bus timeout"), "Should show payload text");
        assert!(text.contains("Logs: 1/1"), "Should show log count");
    }

    #[test]
    fn test_sidebar_prioritizes_most_frequent_entries() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        for i in 0..10 {
            app.logs.push(LogEntry::new(
                DltMessage::new(
                    i,
                    "ECU1".to_string(),
                    None,
                    Some(format!("C{i:03}")),
                    None,
                    b"low frequency".to_vec(),
                ),
                Some(PathBuf::from(format!("source-{i:03}.dlt"))),
                i as usize,
            ));
        }
        for i in 0..2 {
            app.logs.push(LogEntry::new(
                DltMessage::new(
                    10 + i,
                    "ECU1".to_string(),
                    None,
                    Some("ZBUS".to_string()),
                    None,
                    b"high frequency".to_vec(),
                ),
                Some(PathBuf::from("z-busy.dlt")),
                10,
            ));
        }
        app.apply_filter();

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text = buffer_to_string(&terminal);

        assert!(text.contains("    2 z-busy.dlt"));
        assert!(text.contains("    2 ZBUS"));
    }

    #[test]
    fn test_visible_log_range_tracks_selection() {
        assert_eq!(visible_log_range(0, 0, 10), 0..0);
        assert_eq!(visible_log_range(100, 0, 10), 0..10);
        assert_eq!(visible_log_range(100, 9, 10), 0..10);
        assert_eq!(visible_log_range(100, 10, 10), 1..11);
        assert_eq!(visible_log_range(100, 99, 10), 90..100);
    }

    #[test]
    fn test_timestamp_formatting_and_deltas_respect_clock_domains() {
        use crate::parser::TimestampDomain;

        let mut relative = DltMessage::new(12_300, "ECU1".into(), None, None, None, vec![]);
        relative.timestamp_domain = TimestampDomain::DaemonRelative;
        assert_eq!(format_message_timestamp(&relative), "T+123 ticks");

        let mut absolute = DltMessage::new(10_000, "ECU1".into(), None, None, None, vec![]);
        absolute.timestamp_domain = TimestampDomain::StorageAbsolute;
        assert_eq!(format_delta_time(&relative, Some(&absolute), true), "N/A");

        let previous_wrap = u64::from(u32::MAX - 10) * 100;
        let mut previous = DltMessage::new(previous_wrap, "ECU1".into(), None, None, None, vec![]);
        previous.timestamp_domain = TimestampDomain::DaemonRelative;
        let mut wrapped = DltMessage::new(1_200, "ECU1".into(), None, None, None, vec![]);
        wrapped.timestamp_domain = TimestampDomain::DaemonRelative;
        assert_ne!(format_delta_time(&wrapped, Some(&previous), true), "N/A");

        let mut restarted = DltMessage::new(1_000, "ECU1".into(), None, None, None, vec![]);
        restarted.timestamp_domain = TimestampDomain::DaemonRelative;
        assert_eq!(format_delta_time(&restarted, Some(&relative), true), "N/A");
    }

    #[test]
    fn test_log_viewer_only_renders_selected_window() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        for i in 0..100 {
            let text = format!("message-{i:03}");
            app.logs.push(LogEntry::new(
                DltMessage::new(
                    i + 1,
                    "ECU1".to_string(),
                    None,
                    None,
                    None,
                    text.into_bytes(),
                ),
                None,
                0,
            ));
        }
        app.apply_filter();
        app.logs_selected_index = 99;
        assert!(
            app.logs
                .iter()
                .all(|entry| !entry.message.payload_text_is_initialized())
        );

        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(text.contains("message-099"));
        assert!(!text.contains("message-000"), "{text}");
        let initialized = app
            .logs
            .iter()
            .filter(|entry| entry.message.payload_text_is_initialized())
            .count();
        assert!(initialized > 0);
        assert!(initialized < app.logs.len());
        assert!(!app.logs[0].message.payload_text_is_initialized());
        assert!(app.logs[99].message.payload_text_is_initialized());
    }

    #[test]
    fn test_draw_log_detail_screen() {
        let mut app = App::new();
        app.screen = AppScreen::LogDetail;
        app.logs.push(LogEntry::new(
            DltMessage::new(
                5_000_000,
                "ECU2".to_string(),
                Some("NAV".to_string()),
                Some("GPS1".to_string()),
                Some(crate::parser::LogLevel::Info),
                b"GPS fix acquired".to_vec(),
            ),
            Some(PathBuf::from("nav_gps1.dlt")),
            0,
        ));
        app.apply_filter();

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(
            text.contains("Log Metadata"),
            "Should show metadata section"
        );
        assert!(text.contains("Hex Dump"), "Should show hex dump section");
        assert!(text.contains("ECU2"), "Should show ECU ID in detail");
        assert!(
            text.contains("GPS fix acquired"),
            "Should show payload text"
        );
    }

    #[test]
    fn test_log_detail_renders_only_scrolled_hex_rows() {
        let mut app = App::new();
        app.screen = AppScreen::LogDetail;
        app.logs.push(LogEntry::new(
            DltMessage::new(1, "ECU1".to_string(), None, None, None, vec![b'A'; 4096]),
            None,
            0,
        ));
        app.apply_filter();
        app.detail_scroll = 20;

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(text.contains("00000140"), "{text}");
        assert!(!text.contains("00000000"), "{text}");
        assert!(text.contains("rows 21-"), "{text}");
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

    /// FIXED BUG-5: LogDetail status bar shows "No matching logs" instead of "Log 1/0"
    #[test]
    fn test_draw_log_detail_empty_filter() {
        let mut app = App::new();
        app.screen = AppScreen::LogDetail;
        // Add a log but apply a filter that matches nothing
        app.logs.push(LogEntry::new(
            DltMessage::new(
                1_000_000,
                "ECU1".to_string(),
                Some("APP1".to_string()),
                Some("CTX1".to_string()),
                Some(crate::parser::LogLevel::Info),
                b"test message".to_vec(),
            ),
            None,
            0,
        ));
        // filtered_log_indices is empty (no filter applied to populate it)
        // This simulates the case where a filter removes all results

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(
            text.contains("No matching logs"),
            "Should show 'No matching logs' instead of 'Log 1/0'"
        );
        assert!(
            !text.contains("Log 1/0"),
            "Should NOT show misleading 'Log 1/0'"
        );
    }
}
