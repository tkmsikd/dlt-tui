use crate::explorer::{self, FileEntry};
use crate::fs_reader;
use crate::parser::{self, DltMessage};
use std::path::Path;

#[derive(Debug, PartialEq, Clone)]
pub enum AppScreen {
    Explorer,
    LogViewer,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Filter {
    pub min_level: Option<crate::parser::LogLevel>,
    pub app_id: Option<String>,
    pub ctx_id: Option<String>,
    pub text: Option<String>,
}

pub struct App {
    pub screen: AppScreen,
    pub explorer_items: Vec<FileEntry>,
    pub explorer_selected_index: usize,
    pub logs: Vec<DltMessage>,
    pub filtered_log_indices: Vec<usize>,
    pub logs_selected_index: usize,
    pub filter: Filter,
    pub is_entering_filter: bool,
    pub filter_input: String,
    pub should_quit: bool,
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
            is_entering_filter: false,
            filter_input: String::new(),
            should_quit: false,
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
        self.logs_selected_index = 0;
        self.filter = Filter::default();

        let mut stream = fs_reader::open_dlt_stream(path)?;
        let mut buffer = Vec::new();
        std::io::Read::read_to_end(&mut stream, &mut buffer)?;

        // Basic MVP Parse loop
        let mut input = buffer.as_slice();
        while !input.is_empty() {
            match parser::parse_dlt_message(input) {
                Ok((remaining, msg)) => {
                    self.logs.push(msg);
                    input = remaining;
                }
                Err(_) => {
                    // For MVP: On error, just break out (or try to find next magic number ideally)
                    break;
                }
            }
        }

        self.apply_filter();
        self.screen = AppScreen::LogViewer;
        Ok(())
    }

    pub fn apply_filter(&mut self) {
        self.filtered_log_indices.clear();

        for (idx, log) in self.logs.iter().enumerate() {
            let mut matches = true;

            if let Some(ref min_level) = self.filter.min_level {
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
                    matches = false;
                }
            }

            if let Some(ref text) = self.filter.text
                && !log
                    .payload_text
                    .to_lowercase()
                    .contains(&text.to_lowercase())
            {
                matches = false;
            }

            if let Some(ref app_id) = self.filter.app_id
                && log.apid.as_deref() != Some(app_id.as_str())
            {
                matches = false;
            }

            if let Some(ref ctx_id) = self.filter.ctx_id
                && log.ctid.as_deref() != Some(ctx_id.as_str())
            {
                matches = false;
            }

            if matches {
                self.filtered_log_indices.push(idx);
            }
        }

        self.logs_selected_index = 0;
    }

    pub fn on_home(&mut self) {
        match self.screen {
            AppScreen::Explorer => self.explorer_selected_index = 0,
            AppScreen::LogViewer => self.logs_selected_index = 0,
        }
    }

    pub fn on_end(&mut self) {
        match self.screen {
            AppScreen::Explorer => {
                if !self.explorer_items.is_empty() {
                    self.explorer_selected_index = self.explorer_items.len() - 1;
                }
            }
            AppScreen::LogViewer => {
                if !self.filtered_log_indices.is_empty() {
                    self.logs_selected_index = self.filtered_log_indices.len() - 1;
                }
            }
        }
    }

    pub fn on_tick(&mut self) {
        // Handle tick events for animations or asynchronous checks
    }

    pub fn on_up(&mut self) {
        match self.screen {
            AppScreen::Explorer => {
                if self.explorer_selected_index > 0 {
                    self.explorer_selected_index -= 1;
                }
            }
            AppScreen::LogViewer => {
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
            AppScreen::LogViewer => {
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
            AppScreen::LogViewer => self.screen = AppScreen::Explorer,
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
            is_entering_filter: false,
            filter_input: String::new(),
            should_quit: false,
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
