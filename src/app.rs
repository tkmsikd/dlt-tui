use crate::explorer::{self, FileEntry};
use crate::parser::DltMessage;
use std::path::Path;

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
        }
    }

    pub fn load_directory(&mut self, path: &Path) -> std::io::Result<()> {
        let mut entries = explorer::list_directory(path)?;

        // Add ".." parent directory option if it has a parent
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

        // sort by is_dir (directories first), then by name
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

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

        let (tx, rx) = std::sync::mpsc::channel();
        self.log_receiver = Some(rx);

        let path_buf = path.to_path_buf();
        std::thread::spawn(move || {
            let mut stream = match crate::fs_reader::open_dlt_stream(&path_buf) {
                Ok(s) => s,
                Err(_) => return, // Ignore for now
            };
            let mut buffer = Vec::new();
            if std::io::Read::read_to_end(&mut stream, &mut buffer).is_err() {
                return;
            }

            let mut input = buffer.as_slice();
            while !input.is_empty() {
                match crate::parser::parse_dlt_message(input) {
                    Ok((remaining, msg)) => {
                        let _ = tx.send(msg);
                        input = remaining;
                    }
                    Err(_) => break,
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

        if let Some(ref app_id) = filter.app_id
            && log.apid.as_deref() != Some(app_id.as_str())
        {
            return false;
        }

        if let Some(ref ctx_id) = filter.ctx_id
            && log.ctid.as_deref() != Some(ctx_id.as_str())
        {
            return false;
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
                        self.log_receiver = None;
                        break; // Channel closed, thread finished
                    }
                }
            }

            if added {
                // Determine matches incrementally to avoid full rescan
                for idx in current_len..self.logs.len() {
                    let log = &self.logs[idx];
                    if Self::check_log_against_filter(log, &self.filter, text_regex.as_ref()) {
                        self.filtered_log_indices.push(idx);
                    }
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

    pub fn on_enter(&mut self) {
        // For MVP, just flip state for now
        match self.screen {
            AppScreen::Explorer => self.screen = AppScreen::LogViewer,
            AppScreen::LogViewer => self.screen = AppScreen::LogDetail,
            AppScreen::LogDetail => self.screen = AppScreen::LogViewer,
        }
    }

    pub fn on_key_q(&mut self) {
        self.should_quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn build_mock_app_with_explorer_files() -> App {
        let app = App {
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
        };
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
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;

        // Let's populate mock DltMessages
        for i in 0..5 {
            app.logs.push(DltMessage {
                timestamp_us: 1000 + i,
                ecu_id: format!("ECU{}", i),
                apid: None,
                ctid: None,
                log_level: None,
                payload_text: "Mock Payload".to_string(),
                payload_raw: b"Mock Payload".to_vec(),
            });
        }

        app.apply_filter();

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
}
