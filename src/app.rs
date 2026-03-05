use crate::explorer::{self, FileEntry};
use crate::parser::DltMessage;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, PartialEq, Clone)]
pub enum AppScreen {
    Explorer,
    LogViewer,
    LogDetail,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Filter {
    pub min_level: Option<crate::parser::LogLevel>,
    pub app_id: Option<String>,
    pub ctx_id: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum FilterInputMode {
    Text,
    AppId,
    CtxId,
    MinLevel,
}

pub struct App {
    pub screen: AppScreen,
    pub explorer_items: Vec<FileEntry>,
    pub explorer_selected_index: usize,
    pub logs: Vec<DltMessage>,
    pub filtered_log_indices: Vec<usize>,
    pub logs_selected_index: usize,
    pub filter: Filter,
    pub filter_input_mode: Option<FilterInputMode>,
    pub filter_input: String,
    pub error_message: Option<String>,
    pub should_quit: bool,
    pub log_receiver: Option<std::sync::mpsc::Receiver<DltMessage>>,
    pub is_loading: bool,
    pub connection_info: Option<String>,
    pub auto_scroll: bool,
    pub skipped_bytes: usize,
    skipped_bytes_shared: Option<Arc<AtomicUsize>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            screen: AppScreen::Explorer,
            explorer_items: vec![],
            explorer_selected_index: 0,
            logs: vec![],
            filtered_log_indices: vec![],
            logs_selected_index: 0,
            filter: Filter::default(),
            filter_input_mode: None,
            filter_input: String::new(),
            error_message: None,
            should_quit: false,
            log_receiver: None,
            is_loading: false,
            connection_info: None,
            auto_scroll: false,
            skipped_bytes: 0,
            skipped_bytes_shared: None,
        }
    }

    pub fn load_directory(&mut self, path: &Path) -> std::io::Result<()> {
        let mut entries = explorer::list_directory(path)?;

        // Sort first: directories first, then alphabetically by name
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

        // Insert ".." AFTER sorting so it always stays at position 0
        if let Some(parent) = path.parent() {
            entries.insert(
                0,
                FileEntry {
                    name: "..".to_string(),
                    is_dir: true,
                    path: parent.to_path_buf(),
                },
            );
        }

        self.explorer_items = entries;
        self.explorer_selected_index = 0;
        Ok(())
    }

    pub fn load_file(&mut self, path: &Path) -> std::io::Result<()> {
        self.logs.clear();
        self.filtered_log_indices.clear();
        self.logs_selected_index = 0;
        self.filter = Filter::default();
        self.is_loading = true;
        self.skipped_bytes = 0;

        let (tx, rx) = std::sync::mpsc::channel();
        self.log_receiver = Some(rx);

        let skipped_shared = Arc::new(AtomicUsize::new(0));
        self.skipped_bytes_shared = Some(Arc::clone(&skipped_shared));

        let path_buf = path.to_path_buf();
        std::thread::spawn(move || {
            let mut stream = match crate::fs_reader::open_dlt_stream(&path_buf) {
                Ok(s) => s,
                Err(_) => return,
            };
            let mut buffer = Vec::new();
            if std::io::Read::read_to_end(&mut stream, &mut buffer).is_err() {
                return;
            }

            let (messages, skipped) = crate::parser::parse_all_messages(&buffer);
            skipped_shared.store(skipped, Ordering::Relaxed);
            for msg in messages {
                if tx.send(msg).is_err() {
                    break; // Receiver dropped (app quit)
                }
            }
        });

        self.apply_filter();
        self.screen = AppScreen::LogViewer;
        Ok(())
    }

    fn check_log_against_filter(
        log: &DltMessage,
        filter: &Filter,
        regex: Option<&regex::Regex>,
    ) -> bool {
        if let Some(ref min_level) = filter.min_level {
            // Determine if log_level is severe enough or matches
            // Simplification for MVP: We just check exact equality or we can skip for now
            // Actually, let's just do exact matching or implement a partial ord on LogLevel
            // Since LogLevel isn't Ord yet, we will compare them by converting to an integer.
            let level_val = |l: &crate::parser::LogLevel| match l {
                crate::parser::LogLevel::Fatal => 1,
                crate::parser::LogLevel::Error => 2,
                crate::parser::LogLevel::Warn => 3,
                crate::parser::LogLevel::Info => 4,
                crate::parser::LogLevel::Debug => 5,
                crate::parser::LogLevel::Verbose => 6,
                crate::parser::LogLevel::Unknown(_) => 7,
            };

            let target_val = level_val(min_level);
            let current_val = log.log_level.as_ref().map(level_val).unwrap_or(7);

            if current_val > target_val {
                return false;
            }
        }

        if let Some(ref text) = filter.text {
            if let Some(re) = regex {
                if !re.is_match(&log.payload_text) {
                    return false;
                }
            } else if !log
                .payload_text
                .to_lowercase()
                .contains(&text.to_lowercase())
            {
                return false;
            }
        }

        if let Some(ref app_id) = filter.app_id {
            let matches = log
                .apid
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case(app_id))
                .unwrap_or(false);
            if !matches {
                return false;
            }
        }

        if let Some(ref ctx_id) = filter.ctx_id {
            let matches = log
                .ctid
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case(ctx_id))
                .unwrap_or(false);
            if !matches {
                return false;
            }
        }

        true
    }

    pub fn apply_filter(&mut self) {
        self.filtered_log_indices.clear();

        // Compile regex once if text filter exists
        let text_regex = self.filter.text.as_ref().and_then(|text| {
            regex::RegexBuilder::new(text)
                .case_insensitive(true)
                .build()
                .ok() // If invalid regex, we will fallback to plain string search
        });

        for (idx, log) in self.logs.iter().enumerate() {
            if Self::check_log_against_filter(log, &self.filter, text_regex.as_ref()) {
                self.filtered_log_indices.push(idx);
            }
        }

        self.logs_selected_index = 0;
    }

    pub fn on_home(&mut self) {
        match self.screen {
            AppScreen::Explorer => self.explorer_selected_index = 0,
            AppScreen::LogViewer | AppScreen::LogDetail => self.logs_selected_index = 0,
        }
    }

    pub fn on_end(&mut self) {
        match self.screen {
            AppScreen::Explorer => {
                if !self.explorer_items.is_empty() {
                    self.explorer_selected_index = self.explorer_items.len() - 1;
                }
            }
            AppScreen::LogViewer | AppScreen::LogDetail => {
                if !self.filtered_log_indices.is_empty() {
                    self.logs_selected_index = self.filtered_log_indices.len() - 1;
                }
            }
        }
    }

    pub fn on_tick(&mut self) {
        if let Some(rx) = &self.log_receiver {
            let mut added = false;
            let current_len = self.logs.len();

            let text_regex = self.filter.text.as_ref().and_then(|text| {
                regex::RegexBuilder::new(text)
                    .case_insensitive(true)
                    .build()
                    .ok()
            });

            loop {
                match rx.try_recv() {
                    Ok(msg) => {
                        self.logs.push(msg);
                        added = true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.is_loading = false;
                        if self.connection_info.is_some() {
                            self.connection_info = None;
                        }
                        // Read skipped bytes from the shared atomic
                        if let Some(ref shared) = self.skipped_bytes_shared {
                            self.skipped_bytes = shared.load(Ordering::Relaxed);
                        }
                        self.skipped_bytes_shared = None;
                        self.log_receiver = None;
                        break;
                    }
                }
            }

            if added {
                for idx in current_len..self.logs.len() {
                    let log = &self.logs[idx];
                    if Self::check_log_against_filter(log, &self.filter, text_regex.as_ref()) {
                        self.filtered_log_indices.push(idx);
                    }
                }

                // Auto-scroll: keep cursor at the end when in tail mode
                if self.auto_scroll && !self.filtered_log_indices.is_empty() {
                    self.logs_selected_index = self.filtered_log_indices.len() - 1;
                }
            }
        }
    }

    pub fn on_up(&mut self) {
        match self.screen {
            AppScreen::Explorer => {
                if self.explorer_selected_index > 0 {
                    self.explorer_selected_index -= 1;
                }
            }
            AppScreen::LogViewer | AppScreen::LogDetail => {
                if self.logs_selected_index > 0 {
                    self.logs_selected_index -= 1;
                }
            }
        }
    }

    pub fn on_down(&mut self) {
        match self.screen {
            AppScreen::Explorer => {
                if !self.explorer_items.is_empty()
                    && self.explorer_selected_index < self.explorer_items.len() - 1
                {
                    self.explorer_selected_index += 1;
                }
            }
            AppScreen::LogViewer | AppScreen::LogDetail => {
                if !self.filtered_log_indices.is_empty()
                    && self.logs_selected_index < self.filtered_log_indices.len() - 1
                {
                    self.logs_selected_index += 1;
                }
            }
        }
    }

    /// Move selection down by `page_size` rows (full page scroll)
    pub fn on_page_down(&mut self, page_size: usize) {
        if page_size == 0 {
            return;
        }
        match self.screen {
            AppScreen::Explorer => {
                if !self.explorer_items.is_empty() {
                    let max = self.explorer_items.len() - 1;
                    self.explorer_selected_index =
                        (self.explorer_selected_index + page_size).min(max);
                }
            }
            AppScreen::LogViewer | AppScreen::LogDetail => {
                if !self.filtered_log_indices.is_empty() {
                    let max = self.filtered_log_indices.len() - 1;
                    self.logs_selected_index = (self.logs_selected_index + page_size).min(max);
                }
            }
        }
    }

    /// Move selection up by `page_size` rows (full page scroll)
    pub fn on_page_up(&mut self, page_size: usize) {
        if page_size == 0 {
            return;
        }
        match self.screen {
            AppScreen::Explorer => {
                self.explorer_selected_index =
                    self.explorer_selected_index.saturating_sub(page_size);
            }
            AppScreen::LogViewer | AppScreen::LogDetail => {
                self.logs_selected_index = self.logs_selected_index.saturating_sub(page_size);
            }
        }
    }

    /// Move selection down by half a page
    pub fn on_half_page_down(&mut self, page_size: usize) {
        self.on_page_down(page_size / 2);
    }

    /// Move selection up by half a page
    pub fn on_half_page_up(&mut self, page_size: usize) {
        self.on_page_up(page_size / 2);
    }

    pub fn on_key_q(&mut self) {
        self.should_quit = true;
    }

    /// Handle a key event. `page_size` is the number of visible rows (from terminal height).
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent, page_size: usize) {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Dismiss error on any key press
        if self.error_message.is_some() {
            self.error_message = None;
            return;
        }

        // Filter input mode: capture text input for active filter
        if let Some(mode) = self.filter_input_mode.clone() {
            match key.code {
                KeyCode::Enter => {
                    self.filter_input_mode = None;
                    let input = if self.filter_input.is_empty() {
                        None
                    } else {
                        Some(self.filter_input.clone())
                    };
                    match mode {
                        FilterInputMode::Text => self.filter.text = input,
                        FilterInputMode::AppId => self.filter.app_id = input,
                        FilterInputMode::CtxId => self.filter.ctx_id = input,
                        FilterInputMode::MinLevel => {
                            self.filter.min_level = match self.filter_input.to_lowercase().as_str()
                            {
                                "f" | "fatal" => Some(crate::parser::LogLevel::Fatal),
                                "e" | "error" => Some(crate::parser::LogLevel::Error),
                                "w" | "warn" => Some(crate::parser::LogLevel::Warn),
                                "i" | "info" => Some(crate::parser::LogLevel::Info),
                                "d" | "debug" => Some(crate::parser::LogLevel::Debug),
                                "v" | "verbose" => Some(crate::parser::LogLevel::Verbose),
                                _ => None,
                            };
                        }
                    }
                    self.apply_filter();
                }
                KeyCode::Esc => {
                    // Cancel: exit input mode without changing the filter
                    self.filter_input_mode = None;
                    self.filter_input.clear();
                }
                KeyCode::Char(c) => self.filter_input.push(c),
                KeyCode::Backspace => {
                    self.filter_input.pop();
                }
                _ => {}
            }
            return;
        }

        // Normal mode key handling
        match key.code {
            KeyCode::Char('q') => match self.screen {
                AppScreen::Explorer => self.on_key_q(),
                AppScreen::LogViewer => self.screen = AppScreen::Explorer,
                AppScreen::LogDetail => self.screen = AppScreen::LogViewer,
            },
            KeyCode::Char('j') | KeyCode::Down => self.on_down(),
            KeyCode::Char('k') | KeyCode::Up => self.on_up(),
            KeyCode::Char('g') | KeyCode::Home => self.on_home(),
            KeyCode::Char('G') | KeyCode::End => self.on_end(),
            // Page scrolling
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.on_page_down(page_size);
            }
            KeyCode::PageDown => self.on_page_down(page_size),
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.on_page_up(page_size);
            }
            KeyCode::PageUp => self.on_page_up(page_size),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.on_half_page_down(page_size);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.on_half_page_up(page_size);
            }
            KeyCode::Char('/') if self.screen == AppScreen::LogViewer => {
                self.filter_input_mode = Some(FilterInputMode::Text);
                self.filter_input.clear();
            }
            KeyCode::Char('l') if self.screen == AppScreen::LogViewer => {
                self.filter_input_mode = Some(FilterInputMode::MinLevel);
                self.filter_input.clear();
            }
            KeyCode::Char('a') if self.screen == AppScreen::LogViewer => {
                self.filter_input_mode = Some(FilterInputMode::AppId);
                self.filter_input.clear();
            }
            KeyCode::Char('c') if self.screen == AppScreen::LogViewer => {
                self.filter_input_mode = Some(FilterInputMode::CtxId);
                self.filter_input.clear();
            }
            KeyCode::Char('C') if self.screen == AppScreen::LogViewer => {
                self.filter = Filter::default();
                self.apply_filter();
            }
            KeyCode::Char('F') if self.screen == AppScreen::LogViewer => {
                self.auto_scroll = !self.auto_scroll;
            }
            KeyCode::Enter => {
                if self.screen == AppScreen::Explorer {
                    if !self.explorer_items.is_empty() {
                        let selected = &self.explorer_items[self.explorer_selected_index];
                        if selected.is_dir {
                            let path_clone = selected.path.clone();
                            if let Err(e) = self.load_directory(&path_clone) {
                                self.error_message =
                                    Some(format!("Could not open directory: {}", e));
                            }
                        } else {
                            let path_clone = selected.path.clone();
                            if let Err(e) = self.load_file(&path_clone) {
                                self.error_message = Some(format!("Could not open file: {}", e));
                            }
                        }
                    }
                } else if self.screen == AppScreen::LogViewer
                    && !self.filtered_log_indices.is_empty()
                {
                    self.screen = AppScreen::LogDetail;
                }
            }
            KeyCode::Esc => {
                if self.screen == AppScreen::LogViewer {
                    self.screen = AppScreen::Explorer;
                } else if self.screen == AppScreen::LogDetail {
                    self.screen = AppScreen::LogViewer;
                }
            }
            _ => {}
        }
    }

    pub fn connect_tcp(&mut self, addr: &str) {
        self.logs.clear();
        self.filtered_log_indices.clear();
        self.logs_selected_index = 0;
        self.filter = Filter::default();
        self.is_loading = true;
        self.auto_scroll = true;
        self.connection_info = Some(addr.to_string());

        let (tx, rx) = std::sync::mpsc::channel();
        self.log_receiver = Some(rx);

        let addr_owned = addr.to_string();
        std::thread::spawn(move || {
            if let Err(_e) = crate::tcp_client::stream_from_tcp(&addr_owned, tx) {
                // Connection failed — channel will be dropped, on_tick handles it
            }
        });

        self.screen = AppScreen::LogViewer;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn build_mock_app_with_explorer_files() -> App {
        App {
            screen: AppScreen::Explorer,
            explorer_items: vec![
                FileEntry {
                    name: "folder1".to_string(),
                    is_dir: true,
                    path: PathBuf::from("folder1"),
                },
                FileEntry {
                    name: "fileA.dlt".to_string(),
                    is_dir: false,
                    path: PathBuf::from("fileA.dlt"),
                },
                FileEntry {
                    name: "fileB.dlt".to_string(),
                    is_dir: false,
                    path: PathBuf::from("fileB.dlt"),
                },
            ],
            explorer_selected_index: 0,
            logs: vec![],
            filtered_log_indices: vec![],
            logs_selected_index: 0,
            filter: Filter::default(),
            filter_input_mode: None,
            filter_input: String::new(),
            error_message: None,
            should_quit: false,
            log_receiver: None,
            is_loading: false,
            connection_info: None,
            auto_scroll: false,
            skipped_bytes: 0,
            skipped_bytes_shared: None,
        }
    }

    fn build_mock_app_with_logs(count: usize) -> App {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        for i in 0..count {
            app.logs.push(DltMessage {
                timestamp_us: 1000 + i as u64,
                ecu_id: format!("ECU{}", i),
                apid: None,
                ctid: None,
                log_level: None,
                payload_text: format!("Log message {}", i),
                payload_raw: format!("Log message {}", i).into_bytes(),
            });
        }
        app.apply_filter();
        app
    }

    #[test]
    fn test_app_initialization() {
        let app = App::new();
        assert_eq!(app.screen, AppScreen::Explorer);
        assert_eq!(app.explorer_selected_index, 0);
        assert_eq!(app.logs_selected_index, 0);
        assert!(!app.should_quit);
    }

    #[test]
    fn test_quit_on_q() {
        let mut app = App::new();
        app.on_key_q();
        assert!(app.should_quit);
    }

    #[test]
    fn test_explorer_up_down_bounds() {
        let mut app = build_mock_app_with_explorer_files();

        // Initial state index: 0. Trying to go UP shouldn't underflow.
        app.on_up();
        assert_eq!(app.explorer_selected_index, 0);

        // Move down within bounds
        app.on_down();
        assert_eq!(app.explorer_selected_index, 1);

        app.on_down();
        assert_eq!(app.explorer_selected_index, 2);

        // Move down out of bounds, should cap at length - 1 (i.e. 2)
        app.on_down();
        assert_eq!(app.explorer_selected_index, 2);

        // Move back up
        app.on_up();
        assert_eq!(app.explorer_selected_index, 1);
    }

    #[test]
    fn test_log_viewer_up_down_bounds() {
        let mut app = build_mock_app_with_logs(5);

        // Test list traversal for logs
        app.on_up();
        assert_eq!(app.logs_selected_index, 0);

        app.on_down();
        assert_eq!(app.logs_selected_index, 1);

        // move multiple down
        app.on_down();
        app.on_down();
        app.on_down();
        app.on_down();
        // Capped at 4
        assert_eq!(app.logs_selected_index, 4);
        // test home and end
        app.on_home();
        assert_eq!(app.logs_selected_index, 0);

        app.on_end();
        assert_eq!(app.logs_selected_index, 4);
    }

    // ==================== Page scrolling tests ====================

    #[test]
    fn test_page_down_log_viewer() {
        let mut app = build_mock_app_with_logs(100);
        assert_eq!(app.logs_selected_index, 0);

        // Full page down
        app.on_page_down(20);
        assert_eq!(app.logs_selected_index, 20);

        // Another page
        app.on_page_down(20);
        assert_eq!(app.logs_selected_index, 40);
    }

    #[test]
    fn test_page_down_clamps_at_end() {
        let mut app = build_mock_app_with_logs(50);
        app.logs_selected_index = 45;

        // Page down should clamp to last item (index 49)
        app.on_page_down(20);
        assert_eq!(app.logs_selected_index, 49);
    }

    #[test]
    fn test_page_up_log_viewer() {
        let mut app = build_mock_app_with_logs(100);
        app.logs_selected_index = 50;

        app.on_page_up(20);
        assert_eq!(app.logs_selected_index, 30);

        app.on_page_up(20);
        assert_eq!(app.logs_selected_index, 10);
    }

    #[test]
    fn test_page_up_clamps_at_start() {
        let mut app = build_mock_app_with_logs(100);
        app.logs_selected_index = 5;

        // Page up should clamp to 0
        app.on_page_up(20);
        assert_eq!(app.logs_selected_index, 0);
    }

    #[test]
    fn test_half_page_scroll() {
        let mut app = build_mock_app_with_logs(100);

        app.on_half_page_down(20); // 20/2 = 10
        assert_eq!(app.logs_selected_index, 10);

        app.on_half_page_down(20);
        assert_eq!(app.logs_selected_index, 20);

        app.on_half_page_up(20);
        assert_eq!(app.logs_selected_index, 10);

        app.on_half_page_up(20);
        assert_eq!(app.logs_selected_index, 0);
    }

    #[test]
    fn test_page_scroll_explorer() {
        let mut app = build_mock_app_with_explorer_files();
        // Explorer has 3 items (indices 0, 1, 2)

        app.on_page_down(10);
        assert_eq!(app.explorer_selected_index, 2); // clamped

        app.on_page_up(10);
        assert_eq!(app.explorer_selected_index, 0); // clamped
    }

    #[test]
    fn test_page_scroll_empty_list() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        // No logs

        app.on_page_down(20);
        assert_eq!(app.logs_selected_index, 0);

        app.on_page_up(20);
        assert_eq!(app.logs_selected_index, 0);
    }

    #[test]
    fn test_page_scroll_zero_page_size() {
        let mut app = build_mock_app_with_logs(10);
        app.logs_selected_index = 5;

        app.on_page_down(0);
        assert_eq!(app.logs_selected_index, 5); // unchanged

        app.on_page_up(0);
        assert_eq!(app.logs_selected_index, 5); // unchanged
    }

    // ==================== Filter scenario tests ====================

    fn build_mock_app_with_diverse_logs() -> App {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;

        let entries = vec![
            (
                "ECU1",
                Some("DIAG"),
                Some("CAN1"),
                Some(crate::parser::LogLevel::Error),
                "CAN bus timeout on channel 1",
            ),
            (
                "ECU1",
                Some("DIAG"),
                Some("CAN2"),
                Some(crate::parser::LogLevel::Warn),
                "CAN retransmit count high",
            ),
            (
                "ECU1",
                Some("SYS"),
                Some("BOOT"),
                Some(crate::parser::LogLevel::Info),
                "System boot complete",
            ),
            (
                "ECU2",
                Some("NAV"),
                Some("GPS1"),
                Some(crate::parser::LogLevel::Debug),
                "GPS fix acquired lat=35.6 lon=139.7",
            ),
            (
                "ECU2",
                Some("NAV"),
                Some("MAP1"),
                Some(crate::parser::LogLevel::Info),
                "Map data loaded",
            ),
            (
                "ECU1",
                Some("SYS"),
                Some("BOOT"),
                Some(crate::parser::LogLevel::Fatal),
                "Watchdog reset detected",
            ),
            (
                "ECU1",
                Some("DIAG"),
                Some("UDS1"),
                Some(crate::parser::LogLevel::Info),
                "UDS session started",
            ),
            (
                "ECU2",
                Some("HMI"),
                Some("DISP"),
                Some(crate::parser::LogLevel::Verbose),
                "Frame rendered in 16ms",
            ),
        ];

        for (i, (ecu, apid, ctid, level, text)) in entries.into_iter().enumerate() {
            app.logs.push(DltMessage {
                timestamp_us: 1000 + i as u64,
                ecu_id: ecu.to_string(),
                apid: apid.map(|s| s.to_string()),
                ctid: ctid.map(|s| s.to_string()),
                log_level: level,
                payload_text: text.to_string(),
                payload_raw: text.as_bytes().to_vec(),
            });
        }
        app.apply_filter();
        app
    }

    #[test]
    fn test_filter_by_text() {
        let mut app = build_mock_app_with_diverse_logs();
        assert_eq!(app.filtered_log_indices.len(), 8); // all

        app.filter.text = Some("CAN".to_string());
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 2); // "CAN bus timeout" + "CAN retransmit"
    }

    #[test]
    fn test_filter_by_text_case_insensitive() {
        let mut app = build_mock_app_with_diverse_logs();

        app.filter.text = Some("can".to_string());
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 2); // case-insensitive
    }

    #[test]
    fn test_filter_by_app_id() {
        let mut app = build_mock_app_with_diverse_logs();

        app.filter.app_id = Some("DIAG".to_string());
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 3); // CAN1, CAN2, UDS1
    }

    #[test]
    fn test_filter_by_ctx_id() {
        let mut app = build_mock_app_with_diverse_logs();

        app.filter.ctx_id = Some("BOOT".to_string());
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 2); // boot complete + watchdog reset
    }

    #[test]
    fn test_filter_by_log_level() {
        let mut app = build_mock_app_with_diverse_logs();

        // Warn and above (Fatal, Error, Warn)
        app.filter.min_level = Some(crate::parser::LogLevel::Warn);
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 3); // Fatal + Error + Warn
    }

    #[test]
    fn test_filter_compound_level_and_app() {
        let mut app = build_mock_app_with_diverse_logs();

        // DIAG logs at Warn or above
        app.filter.app_id = Some("DIAG".to_string());
        app.filter.min_level = Some(crate::parser::LogLevel::Warn);
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 2); // Error + Warn from DIAG
    }

    #[test]
    fn test_filter_compound_all_criteria() {
        let mut app = build_mock_app_with_diverse_logs();

        // DIAG + Error+ + "timeout"
        app.filter.app_id = Some("DIAG".to_string());
        app.filter.min_level = Some(crate::parser::LogLevel::Error);
        app.filter.text = Some("timeout".to_string());
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 1); // Only "CAN bus timeout"
        assert_eq!(
            app.logs[app.filtered_log_indices[0]].payload_text,
            "CAN bus timeout on channel 1"
        );
    }

    #[test]
    fn test_filter_by_regex() {
        let mut app = build_mock_app_with_diverse_logs();

        // Regex: match "lat=... lon=..."
        app.filter.text = Some(r"lat=\d+\.\d+".to_string());
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 1); // GPS fix
    }

    #[test]
    fn test_filter_no_match() {
        let mut app = build_mock_app_with_diverse_logs();

        app.filter.text = Some("NONEXISTENT_STRING".to_string());
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 0);
    }

    #[test]
    fn test_filter_reset() {
        let mut app = build_mock_app_with_diverse_logs();

        app.filter.text = Some("CAN".to_string());
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 2);

        // Reset
        app.filter = Filter::default();
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 8); // all restored
    }

    #[test]
    fn test_filter_resets_selected_index() {
        let mut app = build_mock_app_with_diverse_logs();
        app.logs_selected_index = 5;

        app.filter.text = Some("CAN".to_string());
        app.apply_filter();
        // apply_filter should reset index to 0
        assert_eq!(app.logs_selected_index, 0);
    }

    // ==================== Bug verification tests ====================

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    /// FIXED BUG-3: Esc during filter input now preserves previous filter value
    #[test]
    fn test_esc_preserves_previous_filter() {
        let mut app = build_mock_app_with_diverse_logs();

        // Set an initial text filter
        app.filter.text = Some("CAN".to_string());
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 2);

        // Enter filter input mode for text
        app.handle_key(make_key(KeyCode::Char('/')), 20);
        assert!(app.filter_input_mode.is_some());

        // Type something new
        app.handle_key(make_key(KeyCode::Char('X')), 20);

        // Press Esc to cancel
        app.handle_key(make_key(KeyCode::Esc), 20);

        // Previous "CAN" filter should be preserved
        assert_eq!(
            app.filter.text,
            Some("CAN".to_string()),
            "Esc should preserve the previous filter value"
        );
        assert_eq!(
            app.filtered_log_indices.len(),
            2,
            "Filter results should remain unchanged after Esc"
        );
    }

    /// FIXED BUG-3b: Esc from first-time filter input leaves filter as None
    #[test]
    fn test_esc_first_time_filter_leaves_none() {
        let mut app = build_mock_app_with_diverse_logs();
        assert_eq!(app.filter.text, None);
        assert_eq!(app.filtered_log_indices.len(), 8);

        // Enter filter input mode for text (no previous filter)
        app.handle_key(make_key(KeyCode::Char('/')), 20);
        app.handle_key(make_key(KeyCode::Char('X')), 20);
        app.handle_key(make_key(KeyCode::Esc), 20);

        // Filter should remain None
        assert_eq!(app.filter.text, None);
        assert_eq!(app.filtered_log_indices.len(), 8);
    }

    /// FIXED BUG-4: APP ID filter is now case-insensitive
    #[test]
    fn test_app_id_filter_case_insensitive() {
        let mut app = build_mock_app_with_diverse_logs();

        // Filter with lowercase "diag" should match "DIAG"
        app.filter.app_id = Some("diag".to_string());
        app.apply_filter();
        assert_eq!(
            app.filtered_log_indices.len(),
            3,
            "APP ID filter should be case-insensitive: 'diag' matches 'DIAG'"
        );

        // CTX ID filter should also be case-insensitive
        app.filter.app_id = None;
        app.filter.ctx_id = Some("boot".to_string());
        app.apply_filter();
        assert_eq!(
            app.filtered_log_indices.len(),
            2,
            "CTX ID filter should be case-insensitive: 'boot' matches 'BOOT'"
        );
    }
}
