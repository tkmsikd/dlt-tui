use crate::explorer::{self, FileEntry};
use crate::parser::DltMessage;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

const LOG_CHANNEL_CAPACITY: usize = 256;
const MAX_LOGS_PER_TICK: usize = 1024;
const MAX_FILTER_CONFIG_BYTES: u64 = 1_048_576;
static CONFIG_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, PartialEq, Clone)]
pub enum AppScreen {
    Explorer,
    LogViewer,
    LogDetail,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    pub min_level: Option<crate::parser::LogLevel>,
    pub app_id: Option<String>,
    pub ctx_id: Option<String>,
    #[serde(default)]
    pub ctx_missing: bool,
    pub source: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum FilterInputMode {
    Text,
    AppId,
    CtxId,
    MinLevel,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SidebarFocus {
    #[default]
    Logs,
    Sources,
    Contexts,
}

#[derive(Debug, PartialEq, Clone)]
pub struct LogEntry {
    pub message: DltMessage,
    pub source_file: Option<Arc<PathBuf>>,
    pub source_index: usize,
}

struct ExportJob {
    path: PathBuf,
    indices: Vec<usize>,
    cursor: usize,
    writer: Option<crate::exporter::TextWriter>,
    completed: bool,
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        if !self.completed {
            self.writer.take();
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl LogEntry {
    pub fn new(message: DltMessage, source_file: Option<PathBuf>, source_index: usize) -> Self {
        Self {
            message,
            source_file: source_file.map(Arc::new),
            source_index,
        }
    }

    fn new_with_shared_source(
        message: DltMessage,
        source_file: Option<Arc<PathBuf>>,
        source_index: usize,
    ) -> Self {
        Self {
            message,
            source_file,
            source_index,
        }
    }

    pub fn source_name(&self) -> &str {
        self.source_file
            .as_deref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("-")
    }

    pub fn source_identity(&self) -> String {
        self.source_file
            .as_deref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "-".to_string())
    }
}

pub struct App {
    pub screen: AppScreen,
    pub explorer_items: Vec<FileEntry>,
    pub explorer_selected_index: usize,
    pub logs: Vec<LogEntry>,
    pub filtered_log_indices: Vec<usize>,
    source_count_cache: BTreeMap<String, usize>,
    ctx_count_cache: BTreeMap<String, usize>,
    ctx_missing_count: usize,
    pub logs_selected_index: usize,
    pub sidebar_focus: SidebarFocus,
    pub source_sidebar_selected_index: usize,
    pub ctx_sidebar_selected_index: usize,
    source_sidebar_selected_key: Option<String>,
    ctx_sidebar_selected_key: Option<Option<String>>,
    pub filter: Filter,
    pub filter_input_mode: Option<FilterInputMode>,
    pub filter_input: String,
    pub error_message: Option<String>,
    pub info_message: Option<String>,
    pub should_quit: bool,
    pub log_receiver: Option<std::sync::mpsc::Receiver<LogEntry>>,
    pub is_loading: bool,
    pub connection_info: Option<String>,
    pub auto_scroll: bool,
    pub horizontal_scroll: usize,
    pub detail_scroll: usize,
    pub show_time_delta: bool,
    pub skipped_bytes: usize,
    skipped_bytes_shared: Option<Arc<AtomicUsize>>,
    tcp_error: Option<Arc<Mutex<Option<String>>>>,
    file_errors: Option<Arc<Mutex<Vec<String>>>>,
    /// BUG-1: Generation counter incremented on every apply_filter() call.
    /// on_tick() only appends new indices if filter_generation hasn't changed.
    filter_generation: u64,
    text_regex_cache: Option<(Option<String>, Option<regex::Regex>)>,
    filter_scan_cursor: Option<usize>,
    export_job: Option<ExportJob>,
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
            source_count_cache: BTreeMap::new(),
            ctx_count_cache: BTreeMap::new(),
            ctx_missing_count: 0,
            logs_selected_index: 0,
            sidebar_focus: SidebarFocus::Logs,
            source_sidebar_selected_index: 0,
            ctx_sidebar_selected_index: 0,
            source_sidebar_selected_key: None,
            ctx_sidebar_selected_key: None,
            filter: Filter::default(),
            filter_input_mode: None,
            filter_input: String::new(),
            error_message: None,
            info_message: None,
            should_quit: false,
            log_receiver: None,
            is_loading: false,
            connection_info: None,
            auto_scroll: false,
            horizontal_scroll: 0,
            detail_scroll: 0,
            show_time_delta: false,
            skipped_bytes: 0,
            skipped_bytes_shared: None,
            tcp_error: None,
            file_errors: None,
            filter_generation: 0,
            text_regex_cache: None,
            filter_scan_cursor: None,
            export_job: None,
        }
    }

    pub fn load_filter_config(&mut self) {
        let path = std::path::Path::new(".dlt-tui.toml");
        match std::fs::File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                if let Err(error) = file
                    .take(MAX_FILTER_CONFIG_BYTES + 1)
                    .read_to_end(&mut bytes)
                {
                    self.error_message = Some(format!("Failed to read filter config: {error}"));
                    return;
                }
                if bytes.len() as u64 > MAX_FILTER_CONFIG_BYTES {
                    self.error_message = Some("Config file too large (>1MB)".to_string());
                    return;
                }
                match toml::from_slice::<Filter>(&bytes) {
                    Ok(filter) => {
                        self.filter = filter;
                        self.info_message = Some("Loaded filter from .dlt-tui.toml".to_string());
                    }
                    Err(error) => {
                        self.error_message = Some(format!("Invalid filter config: {error}"));
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                self.error_message = Some(format!("Failed to read filter config: {error}"));
            }
        }
    }

    pub fn save_filter_config(&mut self) {
        if let Ok(content) = toml::to_string(&self.filter) {
            let destination = Path::new(".dlt-tui.toml");
            let temp_path = destination.with_file_name(format!(
                ".dlt-tui.toml.{}.{}.tmp",
                std::process::id(),
                CONFIG_TEMP_ID.fetch_add(1, Ordering::Relaxed)
            ));
            let save_result = (|| {
                let mut temp = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp_path)?;
                std::io::Write::write_all(&mut temp, content.as_bytes())?;
                temp.sync_all()?;
                drop(temp);
                std::fs::rename(&temp_path, destination)
            })();
            match save_result {
                Ok(()) => {
                    self.info_message = Some("Saved filter to .dlt-tui.toml".to_string());
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&temp_path);
                    self.error_message = Some(format!("Failed to save filter config: {error}"));
                }
            }
        } else {
            self.error_message = Some("Failed to serialize filter".to_string());
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

    pub fn load_files(&mut self, paths: Vec<PathBuf>) -> std::io::Result<()> {
        self.cancel_export();
        self.logs.clear();
        self.filtered_log_indices.clear();
        self.logs_selected_index = 0;
        self.error_message = None;
        self.info_message = None;
        self.connection_info = None;
        self.tcp_error = None;
        self.load_filter_config();
        self.is_loading = true;
        self.auto_scroll = false;
        self.horizontal_scroll = 0;
        self.show_time_delta = false;
        self.skipped_bytes = 0;

        let (tx, rx) = std::sync::mpsc::sync_channel(LOG_CHANNEL_CAPACITY);
        self.log_receiver = Some(rx);

        let skipped_shared = Arc::new(AtomicUsize::new(0));
        self.skipped_bytes_shared = Some(Arc::clone(&skipped_shared));

        let file_errors = Arc::new(Mutex::new(Vec::new()));
        self.file_errors = Some(Arc::clone(&file_errors));

        std::thread::spawn(move || {
            for (source_index, path) in paths.into_iter().enumerate() {
                let stream = match crate::fs_reader::open_dlt_stream(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        if let Ok(mut errors) = file_errors.lock() {
                            errors.push(format!("{}: {}", path.display(), e));
                        }
                        continue;
                    }
                };

                let source_file = Arc::new(path.clone());
                let mut receiver_open = true;
                let stream_result = crate::tcp_client::stream_from_reader_with_handler(
                    stream,
                    Arc::clone(&skipped_shared),
                    |message| {
                        let sent = tx
                            .send(LogEntry::new_with_shared_source(
                                message,
                                Some(Arc::clone(&source_file)),
                                source_index,
                            ))
                            .is_ok();
                        receiver_open = sent;
                        sent
                    },
                );
                if !receiver_open {
                    break;
                }
                if let Err(e) = stream_result
                    && let Ok(mut errors) = file_errors.lock()
                {
                    errors.push(format!("{}: {}", path.display(), e));
                }
            }
        });

        self.apply_filter();
        self.screen = AppScreen::LogViewer;
        Ok(())
    }

    fn check_log_against_filter(
        entry: &LogEntry,
        filter: &Filter,
        regex: Option<&regex::Regex>,
    ) -> bool {
        let log = &entry.message;
        if let Some(source) = &filter.source
            && entry.source_identity() != *source
        {
            return false;
        }
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

        if filter.ctx_missing && log.ctid.is_some() {
            return false;
        }
        if !filter.ctx_missing
            && let Some(ref ctx_id) = filter.ctx_id
        {
            let matches = log
                .ctid
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case(ctx_id))
                .unwrap_or(false);
            if !matches {
                return false;
            }
        }

        if let Some(ref text) = filter.text {
            if let Some(re) = regex {
                if !re.is_match(log.payload_text()) {
                    return false;
                }
            } else if !log
                .payload_text()
                .to_lowercase()
                .contains(&text.to_lowercase())
            {
                return false;
            }
        }

        true
    }

    fn compiled_text_regex(&mut self) -> Option<regex::Regex> {
        let pattern = self.filter.text.clone();
        if self
            .text_regex_cache
            .as_ref()
            .is_none_or(|(cached_pattern, _)| *cached_pattern != pattern)
        {
            let compiled = pattern.as_ref().and_then(|text| {
                regex::RegexBuilder::new(text)
                    .case_insensitive(true)
                    .size_limit(256 * 1024)
                    .build()
                    .ok()
            });
            self.text_regex_cache = Some((pattern, compiled));
        }
        self.text_regex_cache
            .as_ref()
            .and_then(|(_, regex)| regex.clone())
    }

    pub fn apply_filter(&mut self) {
        self.begin_filter();
        while self.filter_scan_cursor.is_some() {
            self.process_filter_chunk(usize::MAX);
        }
    }

    pub fn begin_filter(&mut self) {
        self.filtered_log_indices.clear();
        self.source_count_cache.clear();
        self.ctx_count_cache.clear();
        self.ctx_missing_count = 0;

        self.filter_generation = self.filter_generation.wrapping_add(1);
        self.filter_scan_cursor = Some(0);
        self.logs_selected_index = 0;
        if self.logs.len() <= MAX_LOGS_PER_TICK {
            self.process_filter_chunk(MAX_LOGS_PER_TICK);
        }
    }

    fn process_filter_chunk(&mut self, max_items: usize) {
        let Some(start) = self.filter_scan_cursor else {
            return;
        };
        let end = start.saturating_add(max_items).min(self.logs.len());
        let filter = self.filter.clone();
        let text_regex = self.compiled_text_regex();
        let mut matching = Vec::new();

        for idx in start..end {
            if Self::check_log_against_filter(&self.logs[idx], &filter, text_regex.as_ref()) {
                matching.push(idx);
                self.increment_sidebar_counts(idx);
            }
        }

        self.filter_scan_cursor = (end < self.logs.len()).then_some(end);
        self.merge_filtered_indices(matching);
        self.sync_sidebar_selection();
    }

    #[cfg(test)]
    fn sort_filtered_indices(&mut self) {
        let logs = &self.logs;
        self.filtered_log_indices
            .sort_by_key(|&idx| Self::log_sort_key(logs, idx));
    }

    fn merge_filtered_indices(&mut self, mut incoming: Vec<usize>) {
        if incoming.is_empty() {
            return;
        }

        let selected_log = self
            .filtered_log_indices
            .get(self.logs_selected_index)
            .copied();

        let logs = &self.logs;
        if !incoming
            .windows(2)
            .all(|pair| Self::log_sort_key(logs, pair[0]) <= Self::log_sort_key(logs, pair[1]))
        {
            incoming.sort_by_key(|&idx| Self::log_sort_key(logs, idx));
        }

        if self.filtered_log_indices.is_empty()
            || Self::log_sort_key(logs, *self.filtered_log_indices.last().unwrap())
                <= Self::log_sort_key(logs, incoming[0])
        {
            self.filtered_log_indices.extend(incoming);
        } else {
            let first_key = Self::log_sort_key(logs, incoming[0]);
            let split = self
                .filtered_log_indices
                .partition_point(|&idx| Self::log_sort_key(logs, idx) < first_key);
            let tail = self.filtered_log_indices.split_off(split);
            let mut existing = tail.into_iter().peekable();
            let mut incoming = incoming.into_iter().peekable();
            let mut merged = Vec::with_capacity(existing.len() + incoming.len());

            while let (Some(&left), Some(&right)) = (existing.peek(), incoming.peek()) {
                if Self::log_sort_key(logs, left) <= Self::log_sort_key(logs, right) {
                    merged.push(existing.next().unwrap());
                } else {
                    merged.push(incoming.next().unwrap());
                }
            }
            merged.extend(existing);
            merged.extend(incoming);
            self.filtered_log_indices.extend(merged);
        }

        if let Some(selected_log) = selected_log {
            let selected_key = Self::log_sort_key(logs, selected_log);
            self.logs_selected_index = self
                .filtered_log_indices
                .partition_point(|&idx| Self::log_sort_key(logs, idx) < selected_key);
        }
    }

    fn log_sort_key(logs: &[LogEntry], idx: usize) -> (u8, u64, usize, usize) {
        let entry = &logs[idx];
        match entry.message.timestamp_domain {
            crate::parser::TimestampDomain::StorageAbsolute
            | crate::parser::TimestampDomain::Unspecified
                if entry.message.timestamp_us != 0 =>
            {
                (0, entry.message.timestamp_us, entry.source_index, idx)
            }
            crate::parser::TimestampDomain::DaemonRelative => {
                (1, entry.source_index as u64, idx, idx)
            }
            crate::parser::TimestampDomain::StorageAbsolute => (0, 0, entry.source_index, idx),
            crate::parser::TimestampDomain::Missing
            | crate::parser::TimestampDomain::Unspecified => (2, 0, entry.source_index, idx),
        }
    }

    fn increment_sidebar_counts(&mut self, idx: usize) {
        let source = self.logs[idx].source_identity();
        Self::increment_count(&mut self.source_count_cache, &source);
        if let Some(ctx) = self.logs[idx].message.ctid.as_deref() {
            Self::increment_count(&mut self.ctx_count_cache, ctx);
        } else {
            self.ctx_missing_count += 1;
        }
    }

    fn remember_sidebar_selection(&mut self) {
        match self.sidebar_focus {
            SidebarFocus::Sources => {
                self.source_sidebar_selected_key = Self::ranked_keys(&self.source_count_cache)
                    .get(self.source_sidebar_selected_index)
                    .cloned();
            }
            SidebarFocus::Contexts => {
                self.ctx_sidebar_selected_key = self
                    .ranked_ctx_entries()
                    .get(self.ctx_sidebar_selected_index)
                    .map(|(ctx, _)| ctx.clone());
            }
            SidebarFocus::Logs => {}
        }
    }

    fn sync_sidebar_selection(&mut self) {
        let sources = Self::ranked_keys(&self.source_count_cache);
        if let Some(key) = &self.source_sidebar_selected_key {
            if let Some(index) = sources.iter().position(|candidate| candidate == key) {
                self.source_sidebar_selected_index = index;
            } else if self.filter_scan_cursor.is_none() {
                self.source_sidebar_selected_index = self
                    .source_sidebar_selected_index
                    .min(sources.len().saturating_sub(1));
                self.source_sidebar_selected_key =
                    sources.get(self.source_sidebar_selected_index).cloned();
            }
        } else {
            self.source_sidebar_selected_index = self
                .source_sidebar_selected_index
                .min(sources.len().saturating_sub(1));
            self.source_sidebar_selected_key =
                sources.get(self.source_sidebar_selected_index).cloned();
        }

        let contexts = self.ranked_ctx_entries();
        if let Some(key) = &self.ctx_sidebar_selected_key {
            if let Some(index) = contexts.iter().position(|(candidate, _)| candidate == key) {
                self.ctx_sidebar_selected_index = index;
            } else if self.filter_scan_cursor.is_none() {
                self.ctx_sidebar_selected_index = self
                    .ctx_sidebar_selected_index
                    .min(contexts.len().saturating_sub(1));
                self.ctx_sidebar_selected_key = contexts
                    .get(self.ctx_sidebar_selected_index)
                    .map(|(ctx, _)| ctx.clone());
            }
        } else {
            self.ctx_sidebar_selected_index = self
                .ctx_sidebar_selected_index
                .min(contexts.len().saturating_sub(1));
            self.ctx_sidebar_selected_key = contexts
                .get(self.ctx_sidebar_selected_index)
                .map(|(ctx, _)| ctx.clone());
        }
    }

    fn increment_count(counts: &mut BTreeMap<String, usize>, key: &str) {
        if let Some(count) = counts.get_mut(key) {
            *count += 1;
        } else {
            counts.insert(key.to_string(), 1);
        }
    }

    pub fn source_counts(&self) -> &BTreeMap<String, usize> {
        &self.source_count_cache
    }

    pub fn ctx_counts(&self) -> &BTreeMap<String, usize> {
        &self.ctx_count_cache
    }

    pub(crate) fn ranked_ctx_entries(&self) -> Vec<(Option<String>, usize)> {
        let mut entries: Vec<_> = self
            .ctx_count_cache
            .iter()
            .map(|(ctx, count)| (Some(ctx.clone()), *count))
            .collect();
        if self.ctx_missing_count > 0 {
            entries.push((None, self.ctx_missing_count));
        }
        entries.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        entries
    }

    pub fn selected_entry(&self) -> Option<&LogEntry> {
        self.filtered_log_indices
            .get(self.logs_selected_index)
            .map(|&idx| &self.logs[idx])
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

    pub fn on_right(&mut self, amount: usize) {
        if self.screen == AppScreen::LogViewer {
            self.horizontal_scroll = self.horizontal_scroll.saturating_add(amount);
        }
    }

    pub fn on_left(&mut self, amount: usize) {
        if self.screen == AppScreen::LogViewer {
            self.horizontal_scroll = self.horizontal_scroll.saturating_sub(amount);
        }
    }

    pub fn on_tick(&mut self) {
        if let Some(rx) = &self.log_receiver {
            let mut added = false;
            let current_len = self.logs.len();
            let current_gen = self.filter_generation;

            for _ in 0..MAX_LOGS_PER_TICK {
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
                        // BUG-5: Check for TCP connection error from background thread
                        if let Some(ref tcp_err) = self.tcp_error
                            && let Ok(guard) = tcp_err.lock()
                            && let Some(ref err_msg) = *guard
                        {
                            self.error_message =
                                Some(format!("TCP connection failed: {}", err_msg));
                        }
                        self.tcp_error = None;
                        if let Some(file_errors) = self.file_errors.take()
                            && let Ok(errors) = file_errors.lock()
                            && let Some(first) = errors.first()
                        {
                            self.error_message = Some(if errors.len() == 1 {
                                format!("Failed to load {}", first)
                            } else {
                                format!("Failed to load {} files (first: {})", errors.len(), first)
                            });
                        }
                        if self.connection_info.is_some() {
                            // If no error was set and no logs received, show a clean disconnect message
                            if self.error_message.is_none() && self.logs.is_empty() {
                                self.error_message =
                                    Some("TCP connection ended with no data".to_string());
                            }
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
                let text_regex = self.compiled_text_regex();

                // BUG-1: Only append incrementally if filter hasn't been reapplied
                // since we captured current_gen. If it changed, apply_filter()
                // already rebuilt the full index, so skip incremental append.
                if self.filter_generation == current_gen && self.filter_scan_cursor.is_none() {
                    let mut new_filtered_indices =
                        Vec::with_capacity(self.logs.len() - current_len);
                    for idx in current_len..self.logs.len() {
                        if Self::check_log_against_filter(
                            &self.logs[idx],
                            &self.filter,
                            text_regex.as_ref(),
                        ) {
                            new_filtered_indices.push(idx);
                            self.increment_sidebar_counts(idx);
                        }
                    }
                    self.merge_filtered_indices(new_filtered_indices);
                    self.sync_sidebar_selection();
                }

                // Auto-scroll: keep cursor at the end when in tail mode
                if self.auto_scroll && !self.filtered_log_indices.is_empty() {
                    self.logs_selected_index = self.filtered_log_indices.len() - 1;
                }
            }
        }

        if self.filter_scan_cursor.is_some() {
            self.process_filter_chunk(MAX_LOGS_PER_TICK);
        }
        self.process_export_chunk();
    }

    pub fn on_up(&mut self) {
        if self.screen == AppScreen::LogViewer {
            match self.sidebar_focus {
                SidebarFocus::Sources => {
                    self.source_sidebar_selected_index =
                        self.source_sidebar_selected_index.saturating_sub(1);
                    self.remember_sidebar_selection();
                    return;
                }
                SidebarFocus::Contexts => {
                    self.ctx_sidebar_selected_index =
                        self.ctx_sidebar_selected_index.saturating_sub(1);
                    self.remember_sidebar_selection();
                    return;
                }
                SidebarFocus::Logs => {}
            }
        }
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
            AppScreen::LogDetail => {
                if self.logs_selected_index > 0 {
                    self.logs_selected_index -= 1;
                    self.detail_scroll = 0;
                }
            }
        }
    }

    pub fn on_down(&mut self) {
        if self.screen == AppScreen::LogViewer {
            match self.sidebar_focus {
                SidebarFocus::Sources => {
                    self.source_sidebar_selected_index = self
                        .source_sidebar_selected_index
                        .saturating_add(1)
                        .min(self.source_count_cache.len().saturating_sub(1));
                    self.remember_sidebar_selection();
                    return;
                }
                SidebarFocus::Contexts => {
                    self.ctx_sidebar_selected_index = self
                        .ctx_sidebar_selected_index
                        .saturating_add(1)
                        .min(self.ranked_ctx_entries().len().saturating_sub(1));
                    self.remember_sidebar_selection();
                    return;
                }
                SidebarFocus::Logs => {}
            }
        }
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
            AppScreen::LogDetail => {
                if !self.filtered_log_indices.is_empty()
                    && self.logs_selected_index < self.filtered_log_indices.len() - 1
                {
                    self.logs_selected_index += 1;
                    self.detail_scroll = 0;
                }
            }
        }
    }

    fn ranked_keys(counts: &BTreeMap<String, usize>) -> Vec<String> {
        let mut entries: Vec<_> = counts.iter().collect();
        entries.sort_unstable_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        entries.into_iter().map(|(key, _)| key.clone()).collect()
    }

    fn select_sidebar_filter(&mut self) {
        match self.sidebar_focus {
            SidebarFocus::Sources => {
                if let Some(source) = self.source_sidebar_selected_key.as_ref() {
                    self.filter.source = Some(source.clone());
                }
            }
            SidebarFocus::Contexts => {
                if let Some(ctx) = self.ctx_sidebar_selected_key.as_ref() {
                    self.filter.ctx_id = ctx.clone();
                    self.filter.ctx_missing = ctx.is_none();
                }
            }
            SidebarFocus::Logs => return,
        }
        self.sidebar_focus = SidebarFocus::Logs;
        self.begin_filter();
    }

    fn cycle_sidebar_focus(&mut self) {
        self.sidebar_focus = match self.sidebar_focus {
            SidebarFocus::Logs => {
                if let Some(selected) = self.selected_entry() {
                    let selected_key = selected.source_identity();
                    if let Some(position) = Self::ranked_keys(&self.source_count_cache)
                        .iter()
                        .position(|key| *key == selected_key)
                    {
                        self.source_sidebar_selected_index = position;
                        self.source_sidebar_selected_key = Some(selected_key);
                    }
                }
                SidebarFocus::Sources
            }
            SidebarFocus::Sources => {
                if let Some(selected) = self.selected_entry() {
                    let selected_ctx = selected.message.ctid.clone();
                    if let Some(position) = self
                        .ranked_ctx_entries()
                        .iter()
                        .position(|(ctx, _)| *ctx == selected_ctx)
                    {
                        self.ctx_sidebar_selected_index = position;
                        self.ctx_sidebar_selected_key = Some(selected_ctx);
                    }
                }
                SidebarFocus::Contexts
            }
            SidebarFocus::Contexts => SidebarFocus::Logs,
        };
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
            AppScreen::LogViewer => {
                if !self.filtered_log_indices.is_empty() {
                    let max = self.filtered_log_indices.len() - 1;
                    self.logs_selected_index = (self.logs_selected_index + page_size).min(max);
                }
            }
            AppScreen::LogDetail => {
                self.detail_scroll = self.detail_scroll.saturating_add(page_size);
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
            AppScreen::LogViewer => {
                self.logs_selected_index = self.logs_selected_index.saturating_sub(page_size);
            }
            AppScreen::LogDetail => {
                self.detail_scroll = self.detail_scroll.saturating_sub(page_size);
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
        use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};

        // Some terminals report a separate release event. It must not repeat
        // commands, append filter text, or dismiss notifications.
        if key.kind == KeyEventKind::Release {
            return;
        }

        // Dismiss error or info on any key press
        if self.error_message.is_some() || self.info_message.is_some() {
            self.error_message = None;
            self.info_message = None;
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
                    // UX-4: feedback when filter is cleared
                    let was_cleared = input.is_none();
                    match mode {
                        FilterInputMode::Text => self.filter.text = input,
                        FilterInputMode::AppId => self.filter.app_id = input,
                        FilterInputMode::CtxId => {
                            self.filter.ctx_id = input;
                            self.filter.ctx_missing = false;
                        }
                        FilterInputMode::MinLevel => {
                            let level = match self.filter_input.to_lowercase().as_str() {
                                "f" | "fatal" => Some(crate::parser::LogLevel::Fatal),
                                "e" | "error" => Some(crate::parser::LogLevel::Error),
                                "w" | "warn" => Some(crate::parser::LogLevel::Warn),
                                "i" | "info" => Some(crate::parser::LogLevel::Info),
                                "d" | "debug" => Some(crate::parser::LogLevel::Debug),
                                "v" | "verbose" => Some(crate::parser::LogLevel::Verbose),
                                _ => None,
                            };
                            if !was_cleared && level.is_none() {
                                self.error_message =
                                    Some("Invalid level; use F/E/W/I/D/V".to_string());
                                return;
                            }
                            self.filter.min_level = level;
                        }
                    }
                    self.begin_filter();
                    if was_cleared {
                        self.info_message = Some("Filter cleared".to_string());
                    }
                }
                KeyCode::Esc => {
                    // Cancel: exit input mode without changing the filter
                    self.filter_input_mode = None;
                    self.filter_input.clear();
                }
                KeyCode::Char(c) if self.filter_input.len() < 1024 => {
                    // SEC-5: limit filter input length to prevent memory abuse
                    self.filter_input.push(c);
                }
                KeyCode::Char(_) => {}
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
                // UX-6: q on LogDetail goes back to LogViewer
                AppScreen::LogDetail => self.screen = AppScreen::LogViewer,
            },
            KeyCode::Char('b')
                if self.screen == AppScreen::Explorer
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                // BUG-2: bounds check before accessing explorer_items
                if let Some(selected) = self.explorer_items.get(self.explorer_selected_index) {
                    let dir_path = if selected.is_dir {
                        selected.path.clone()
                    } else {
                        selected
                            .path
                            .parent()
                            .unwrap_or(&selected.path)
                            .to_path_buf()
                    };

                    if let Ok(mut entries) = crate::explorer::list_directory(&dir_path) {
                        entries.sort_by(|a, b| a.name.cmp(&b.name));
                        let files: Vec<PathBuf> = entries
                            .into_iter()
                            .filter(|e| {
                                !e.is_dir
                                    && (e.name.ends_with(".dlt")
                                        || e.name.ends_with(".dlt.gz")
                                        || e.name.ends_with(".dlt.zip"))
                            })
                            .map(|e| e.path)
                            .collect();

                        if files.is_empty() {
                            self.error_message =
                                Some("No DLT files found in directory".to_string());
                        } else {
                            self.info_message =
                                Some(format!("Batch loading {} files...", files.len()));
                            if let Err(e) = self.load_files(files) {
                                self.error_message = Some(format!("Batch load failed: {}", e));
                            }
                        }
                    } else {
                        self.error_message =
                            Some("Failed to read directory for batch load".to_string());
                    }
                }
            }
            KeyCode::Char('E') if self.screen == AppScreen::LogViewer => {
                self.on_export();
            }
            KeyCode::Char('S') if self.screen == AppScreen::LogViewer => {
                self.save_filter_config();
            }
            KeyCode::Char('L') if self.screen == AppScreen::LogViewer => {
                self.load_filter_config();
                self.begin_filter();
            }
            KeyCode::Char('t') if self.screen == AppScreen::LogViewer => {
                self.show_time_delta = !self.show_time_delta;
            }
            KeyCode::Char('j') | KeyCode::Down => self.on_down(),
            KeyCode::Char('k') | KeyCode::Up => self.on_up(),
            KeyCode::Left => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.on_left(10);
                } else {
                    self.on_left(1);
                }
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.on_right(10);
                } else {
                    self.on_right(1);
                }
            }
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
                self.begin_filter();
            }
            KeyCode::Char('F') if self.screen == AppScreen::LogViewer => {
                self.auto_scroll = !self.auto_scroll;
            }
            KeyCode::Tab if self.screen == AppScreen::LogViewer => self.cycle_sidebar_focus(),
            KeyCode::Enter => {
                if self.screen == AppScreen::Explorer {
                    // BUG-2: bounds check before accessing explorer_items
                    if let Some(selected) = self.explorer_items.get(self.explorer_selected_index) {
                        if selected.is_dir {
                            let path_clone = selected.path.clone();
                            if let Err(e) = self.load_directory(&path_clone) {
                                self.error_message =
                                    Some(format!("Could not open directory: {}", e));
                            }
                        } else {
                            let path_clone = selected.path.clone();
                            if let Err(e) = self.load_files(vec![path_clone]) {
                                self.error_message = Some(format!("Could not open file: {}", e));
                            }
                        }
                    }
                } else if self.screen == AppScreen::LogViewer {
                    if self.sidebar_focus != SidebarFocus::Logs {
                        self.select_sidebar_filter();
                    } else if !self.filtered_log_indices.is_empty() {
                        self.screen = AppScreen::LogDetail;
                        self.detail_scroll = 0;
                    }
                }
            }
            KeyCode::Esc => {
                // UX-1: Esc consistently means "go back" — on Explorer, quit the app
                if self.screen == AppScreen::Explorer {
                    self.on_key_q();
                } else if self.screen == AppScreen::LogViewer {
                    self.screen = AppScreen::Explorer;
                } else if self.screen == AppScreen::LogDetail {
                    self.screen = AppScreen::LogViewer;
                }
            }
            _ => {}
        }
    }

    pub fn connect_tcp(&mut self, addr: &str) {
        self.cancel_export();
        self.logs.clear();
        self.filtered_log_indices.clear();
        self.source_count_cache.clear();
        self.ctx_count_cache.clear();
        self.logs_selected_index = 0;
        self.error_message = None;
        self.info_message = None;
        self.file_errors = None;
        self.load_filter_config();
        self.begin_filter();
        self.is_loading = true;
        self.auto_scroll = true;
        self.horizontal_scroll = 0;
        self.show_time_delta = false;
        self.skipped_bytes = 0;
        self.connection_info = Some(addr.to_string());

        let (tx, rx) = std::sync::mpsc::sync_channel(LOG_CHANNEL_CAPACITY);
        self.log_receiver = Some(rx);

        let skipped_shared = Arc::new(AtomicUsize::new(0));
        self.skipped_bytes_shared = Some(Arc::clone(&skipped_shared));

        // BUG-5: shared error state so background thread can report connection failures
        let tcp_error = Arc::new(Mutex::new(None));
        self.tcp_error = Some(Arc::clone(&tcp_error));

        let addr_owned = addr.to_string();
        std::thread::spawn(move || {
            if let Err(e) = crate::tcp_client::stream_from_tcp_with_handler(
                &addr_owned,
                skipped_shared,
                |message| tx.send(LogEntry::new(message, None, 0)).is_ok(),
            ) && let Ok(mut guard) = tcp_error.lock()
            {
                *guard = Some(e.to_string());
            }
        });

        self.screen = AppScreen::LogViewer;
    }

    pub fn on_export(&mut self) {
        if self.export_job.is_some() {
            self.info_message = Some("An export is already in progress".to_string());
            return;
        }
        if self.filter_scan_cursor.is_some() {
            self.error_message = Some("Wait for filtering to finish before exporting".to_string());
            return;
        }
        if self.filtered_log_indices.is_empty() {
            self.error_message = Some("No logs to export".to_string());
            return;
        }

        let now = chrono::Local::now();
        let base = format!("dlt_export_{}", now.format("%Y%m%d_%H%M%S"));

        for suffix in 0..=999 {
            let filename = if suffix == 0 {
                format!("{base}.txt")
            } else {
                format!("{base}_{suffix}.txt")
            };
            let path = PathBuf::from(&filename);
            match crate::exporter::TextWriter::create_new(&path) {
                Ok(writer) => {
                    let count = self.filtered_log_indices.len();
                    self.export_job = Some(ExportJob {
                        path,
                        indices: self.filtered_log_indices.clone(),
                        cursor: 0,
                        writer: Some(writer),
                        completed: false,
                    });
                    self.info_message = Some(format!("Exporting {count} logs to {filename}..."));
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    self.error_message = Some(format!("Export failed: {error}"));
                    return;
                }
            }
        }

        self.error_message = Some("Too many export files".to_string());
    }

    fn cancel_export(&mut self) {
        if self.export_job.take().is_some() {
            self.info_message = Some("Export cancelled because the log set changed".to_string());
        }
    }

    fn process_export_chunk(&mut self) {
        let Some(mut job) = self.export_job.take() else {
            return;
        };
        let end = job
            .cursor
            .saturating_add(MAX_LOGS_PER_TICK)
            .min(job.indices.len());
        for &index in &job.indices[job.cursor..end] {
            let Some(entry) = self.logs.get(index) else {
                self.error_message = Some("Export cancelled: log data changed".to_string());
                return;
            };
            if let Err(error) = job
                .writer
                .as_mut()
                .expect("active export has a writer")
                .write_message(&entry.message)
            {
                self.error_message = Some(format!("Export failed: {error}"));
                return;
            }
        }
        job.cursor = end;

        if end == job.indices.len() {
            let count = job.indices.len();
            let filename = job.path.display().to_string();
            match job
                .writer
                .take()
                .expect("active export has a writer")
                .finish()
            {
                Ok(()) => {
                    job.completed = true;
                    self.info_message = Some(format!("Exported {count} logs to {filename}"));
                }
                Err(error) => self.error_message = Some(format!("Export failed: {error}")),
            }
        } else {
            self.info_message = Some(format!(
                "Exporting {} of {} logs...",
                end,
                job.indices.len()
            ));
            self.export_job = Some(job);
        }
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
            ..App::new()
        }
    }

    fn build_mock_app_with_logs(count: usize) -> App {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        for i in 0..count {
            app.logs.push(LogEntry::new(
                DltMessage::new(
                    1000 + i as u64,
                    format!("ECU{}", i),
                    None,
                    None,
                    None,
                    format!("Log message {}", i).into_bytes(),
                ),
                None,
                0,
            ));
        }
        app.apply_filter();
        app
    }

    fn mock_entry(
        timestamp_us: u64,
        payload_text: &str,
        source_file: Option<&str>,
        source_index: usize,
    ) -> LogEntry {
        LogEntry::new(
            DltMessage::new(
                timestamp_us,
                "ECU1".to_string(),
                Some("APP1".to_string()),
                Some("CTX1".to_string()),
                Some(crate::parser::LogLevel::Info),
                payload_text.as_bytes().to_vec(),
            ),
            source_file.map(PathBuf::from),
            source_index,
        )
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

    #[test]
    fn test_apply_filter_orders_logs_by_timestamp_across_sources() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        app.logs
            .push(mock_entry(3_000, "third", Some("ctx_a.dlt"), 0));
        app.logs
            .push(mock_entry(1_000, "first", Some("ctx_b.dlt"), 1));
        app.logs
            .push(mock_entry(2_000, "second", Some("ctx_a.dlt"), 0));

        app.apply_filter();

        let ordered_payloads: Vec<&str> = app
            .filtered_log_indices
            .iter()
            .map(|&idx| app.logs[idx].message.payload_text())
            .collect();
        assert_eq!(ordered_payloads, vec!["first", "second", "third"]);
    }

    #[test]
    fn test_merge_orders_timestamp_domains_without_reordering_restarts() {
        use crate::parser::TimestampDomain;

        let mut app = App::new();
        let mut relative_late = mock_entry(900_000, "relative late", None, 1);
        relative_late.message.timestamp_domain = TimestampDomain::DaemonRelative;
        let mut absolute_late = mock_entry(200, "absolute late", None, 0);
        absolute_late.message.timestamp_domain = TimestampDomain::StorageAbsolute;
        let mut relative_restart = mock_entry(10, "relative restart", None, 1);
        relative_restart.message.timestamp_domain = TimestampDomain::DaemonRelative;
        let mut absolute_early = mock_entry(100, "absolute early", None, 0);
        absolute_early.message.timestamp_domain = TimestampDomain::StorageAbsolute;
        app.logs = vec![
            relative_late,
            absolute_late,
            relative_restart,
            absolute_early,
        ];

        app.apply_filter();

        assert_eq!(app.filtered_log_indices, vec![3, 1, 0, 2]);
    }

    #[test]
    fn test_merge_filtered_indices_interleaves_sorted_batches() {
        let mut app = App::new();
        app.logs.push(mock_entry(100, "first", None, 0));
        app.logs.push(mock_entry(300, "third", None, 0));
        app.logs.push(mock_entry(400, "fourth", None, 0));
        app.logs.push(mock_entry(200, "second", None, 0));
        app.filtered_log_indices = vec![0, 1];

        app.merge_filtered_indices(vec![2, 3]);

        assert_eq!(app.filtered_log_indices, vec![0, 3, 1, 2]);
    }

    #[test]
    fn test_merge_filtered_indices_matches_full_sort() {
        let mut app = App::new();
        for i in 0..500 {
            app.logs.push(mock_entry(
                ((i * 73) % 499 + 1) as u64,
                &format!("message {i}"),
                None,
                i % 3,
            ));
        }

        app.filtered_log_indices = (0..300).collect();
        app.sort_filtered_indices();
        let incoming: Vec<usize> = (300..500).rev().collect();
        let mut expected: Vec<usize> = (0..500).collect();
        expected.sort_by_key(|&idx| App::log_sort_key(&app.logs, idx));

        app.merge_filtered_indices(incoming);

        assert_eq!(app.filtered_log_indices, expected);
    }

    #[test]
    fn test_merge_filtered_indices_keeps_zero_timestamp_last() {
        let mut app = App::new();
        app.logs.push(mock_entry(100, "source one", None, 1));
        app.logs.push(mock_entry(100, "source zero", None, 0));
        app.logs.push(mock_entry(0, "unknown time", None, 0));
        app.filtered_log_indices = vec![0];

        app.merge_filtered_indices(vec![2, 1]);

        assert_eq!(app.filtered_log_indices, vec![1, 0, 2]);
    }

    #[test]
    fn test_merge_filtered_indices_preserves_selected_log() {
        let mut app = App::new();
        app.logs.push(mock_entry(200, "selected", None, 0));
        app.logs.push(mock_entry(100, "earlier", None, 1));
        app.filtered_log_indices = vec![0];

        app.merge_filtered_indices(vec![1]);

        assert_eq!(app.logs_selected_index, 1);
        assert_eq!(
            app.selected_entry().unwrap().message.payload_text(),
            "selected"
        );
    }

    #[test]
    #[ignore = "manual benchmark for large overlapping timestamp batches"]
    fn benchmark_merge_filtered_indices_overlapping_ranges() {
        use std::time::Instant;

        const EXISTING: usize = 100_000;
        const INCOMING: usize = 10_000;
        const ITERATIONS: usize = 20;

        let mut app = App::new();
        app.logs.reserve(EXISTING + INCOMING);
        for i in 0..EXISTING {
            app.logs
                .push(mock_entry((i * 2) as u64, "existing", None, 0));
        }
        for i in 0..INCOMING {
            app.logs
                .push(mock_entry((EXISTING + i * 2) as u64, "incoming", None, 1));
        }
        let base_indices: Vec<usize> = (0..EXISTING).collect();
        let incoming: Vec<usize> = (EXISTING..EXISTING + INCOMING).collect();

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            app.filtered_log_indices.clone_from(&base_indices);
            app.logs_selected_index = EXISTING / 2;
            app.merge_filtered_indices(incoming.clone());
            assert_eq!(app.filtered_log_indices.len(), EXISTING + INCOMING);
        }
        eprintln!(
            "{} overlapping merges of {}+{} indices: {:?}",
            ITERATIONS,
            EXISTING,
            INCOMING,
            started.elapsed()
        );
    }

    #[test]
    fn test_log_entry_source_name_uses_file_name() {
        let entry = mock_entry(1_000, "payload", Some("/tmp/logs/ctx_a.dlt"), 0);

        assert_eq!(entry.source_name(), "ctx_a.dlt");
    }

    #[test]
    fn test_log_entries_share_source_path_allocation() {
        let source = Arc::new(PathBuf::from("/tmp/logs/shared.dlt"));
        let first = LogEntry::new_with_shared_source(
            mock_entry(1_000, "first", None, 0).message,
            Some(Arc::clone(&source)),
            0,
        );
        let second = LogEntry::new_with_shared_source(
            mock_entry(2_000, "second", None, 0).message,
            Some(Arc::clone(&source)),
            0,
        );

        assert!(Arc::ptr_eq(
            first.source_file.as_ref().unwrap(),
            second.source_file.as_ref().unwrap()
        ));
    }

    #[test]
    fn test_source_and_ctx_counts_use_filtered_logs() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        app.logs
            .push(mock_entry(1_000, "keep one", Some("ctx_a.dlt"), 0));
        app.logs
            .push(mock_entry(2_000, "drop me", Some("ctx_b.dlt"), 1));
        app.logs
            .push(mock_entry(3_000, "keep two", Some("ctx_a.dlt"), 0));
        app.filter.text = Some("keep".to_string());

        app.apply_filter();

        assert_eq!(app.source_counts().len(), 1);
        assert_eq!(app.source_counts().get("ctx_a.dlt"), Some(&2));
        assert_eq!(app.ctx_counts().len(), 1);
        assert_eq!(app.ctx_counts().get("CTX1"), Some(&2));

        app.filter.text = Some("drop".to_string());
        app.apply_filter();

        assert_eq!(app.source_counts().len(), 1);
        assert_eq!(app.source_counts().get("ctx_b.dlt"), Some(&1));
        assert_eq!(app.ctx_counts().get("CTX1"), Some(&1));
    }

    #[test]
    fn test_source_filter_distinguishes_matching_basenames() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        app.logs
            .push(mock_entry(1, "first", Some("left/trace.dlt"), 0));
        app.logs
            .push(mock_entry(2, "second", Some("right/trace.dlt"), 1));
        app.apply_filter();
        assert_eq!(app.source_counts().len(), 2);

        app.handle_key(make_key(KeyCode::Tab), 20);
        app.handle_key(make_key(KeyCode::Down), 20);
        app.handle_key(make_key(KeyCode::Enter), 20);

        assert_eq!(app.filtered_log_indices.len(), 1);
        assert_eq!(
            app.logs[app.filtered_log_indices[0]].source_identity(),
            "right/trace.dlt"
        );
    }

    #[test]
    fn test_missing_context_filter_is_distinct_from_dash_context() {
        let mut app = App::new();
        let mut missing = mock_entry(1, "missing", None, 0);
        missing.message.ctid = None;
        let mut dash = mock_entry(2, "dash", None, 0);
        dash.message.ctid = Some("-".to_string());
        app.logs = vec![missing, dash];
        app.screen = AppScreen::LogViewer;
        app.apply_filter();

        assert_eq!(app.ctx_missing_count, 1);
        assert_eq!(app.ctx_counts().get("-"), Some(&1));

        app.sidebar_focus = SidebarFocus::Contexts;
        app.ctx_sidebar_selected_index = 0;
        app.select_sidebar_filter();
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 1);
        assert!(app.logs[app.filtered_log_indices[0]].message.ctid.is_none());

        app.filter = Filter::default();
        app.apply_filter();
        app.sidebar_focus = SidebarFocus::Contexts;
        app.ctx_sidebar_selected_index = 1;
        app.select_sidebar_filter();
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 1);
        assert_eq!(
            app.logs[app.filtered_log_indices[0]]
                .message
                .ctid
                .as_deref(),
            Some("-")
        );
    }

    #[test]
    fn test_sidebar_navigation_reaches_entries_beyond_initial_view() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        for i in 0..20 {
            app.logs.push(mock_entry(
                i as u64,
                "entry",
                Some(&format!("source-{i:02}.dlt")),
                i,
            ));
        }
        app.apply_filter();
        app.sidebar_focus = SidebarFocus::Sources;
        for _ in 0..19 {
            app.on_down();
        }

        assert_eq!(app.source_sidebar_selected_index, 19);
        app.on_down();
        assert_eq!(app.source_sidebar_selected_index, 19);
    }

    #[test]
    fn test_sidebar_selection_tracks_identity_when_counts_change() {
        let mut app = App::new();
        app.screen = AppScreen::LogViewer;
        for i in 0..10 {
            let mut entry = mock_entry(i, "source A", Some("source-a.dlt"), 0);
            entry.message.ctid = Some("CTX_A".to_string());
            app.logs.push(entry);
        }
        for i in 0..9 {
            let mut entry = mock_entry(10 + i, "source B", Some("source-b.dlt"), 1);
            entry.message.ctid = Some("CTX_B".to_string());
            app.logs.push(entry);
        }
        app.apply_filter();

        app.sidebar_focus = SidebarFocus::Sources;
        app.source_sidebar_selected_index = 1;
        app.remember_sidebar_selection();
        app.sidebar_focus = SidebarFocus::Contexts;
        app.ctx_sidebar_selected_index = 1;
        app.remember_sidebar_selection();

        let (tx, rx) = std::sync::mpsc::channel();
        for i in 0..2 {
            let mut entry = mock_entry(20 + i, "source B new", Some("source-b.dlt"), 1);
            entry.message.ctid = Some("CTX_B".to_string());
            tx.send(entry).unwrap();
        }
        drop(tx);
        app.log_receiver = Some(rx);
        app.on_tick();

        assert_eq!(app.source_sidebar_selected_index, 0);
        assert_eq!(app.ctx_sidebar_selected_index, 0);
        assert_eq!(
            app.source_sidebar_selected_key.as_deref(),
            Some("source-b.dlt")
        );
        assert_eq!(
            app.ctx_sidebar_selected_key,
            Some(Some("CTX_B".to_string()))
        );
        app.sidebar_focus = SidebarFocus::Contexts;
        app.select_sidebar_filter();
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 11);
        assert!(app.filtered_log_indices.iter().all(|&idx| {
            app.logs[idx].message.ctid.as_deref() == Some("CTX_B")
                && app.logs[idx].source_name() == "source-b.dlt"
        }));

        app.filter = Filter::default();
        app.apply_filter();
        app.sidebar_focus = SidebarFocus::Sources;
        app.select_sidebar_filter();
        app.apply_filter();
        assert_eq!(app.filtered_log_indices.len(), 11);
        assert!(
            app.filtered_log_indices
                .iter()
                .all(|&idx| { app.logs[idx].source_name() == "source-b.dlt" })
        );
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
            app.logs.push(LogEntry::new(
                DltMessage::new(
                    1000 + i as u64,
                    ecu.to_string(),
                    apid.map(|s| s.to_string()),
                    ctid.map(|s| s.to_string()),
                    level,
                    text.as_bytes().to_vec(),
                ),
                None,
                0,
            ));
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
    fn test_metadata_filters_do_not_materialize_payload_text() {
        let mut app = build_mock_app_with_diverse_logs();
        assert!(
            app.logs
                .iter()
                .all(|entry| !entry.message.payload_text_is_initialized())
        );

        app.filter.app_id = Some("DIAG".to_string());
        app.filter.ctx_id = Some("CAN1".to_string());
        app.filter.min_level = Some(crate::parser::LogLevel::Error);
        app.apply_filter();

        assert_eq!(app.filtered_log_indices.len(), 1);
        assert!(
            app.logs
                .iter()
                .all(|entry| !entry.message.payload_text_is_initialized())
        );
    }

    #[test]
    fn test_metadata_filters_run_before_text_materialization() {
        let mut app = build_mock_app_with_diverse_logs();
        app.filter.app_id = Some("HMI".to_string());
        app.filter.text = Some("Frame".to_string());
        app.apply_filter();

        assert_eq!(app.filtered_log_indices.len(), 1);
        assert_eq!(
            app.logs
                .iter()
                .filter(|entry| entry.message.payload_text_is_initialized())
                .count(),
            1
        );
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
            app.logs[app.filtered_log_indices[0]].message.payload_text(),
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

    #[test]
    fn test_horizontal_scroll() {
        let mut app = build_mock_app_with_logs(1);
        assert_eq!(app.horizontal_scroll, 0);

        app.on_right(1);
        assert_eq!(app.horizontal_scroll, 1);

        app.on_right(10);
        assert_eq!(app.horizontal_scroll, 11);

        app.on_left(5);
        assert_eq!(app.horizontal_scroll, 6);

        app.on_left(10);
        assert_eq!(app.horizontal_scroll, 0, "Should clamp at 0");
    }

    #[test]
    fn test_detail_page_keys_scroll_content_and_reset_on_log_change() {
        let mut app = build_mock_app_with_logs(3);
        app.screen = AppScreen::LogDetail;

        app.on_page_down(5);
        assert_eq!(app.detail_scroll, 5);
        app.on_page_up(2);
        assert_eq!(app.detail_scroll, 3);
        app.on_down();
        assert_eq!(app.logs_selected_index, 1);
        assert_eq!(app.detail_scroll, 0);
    }

    // ==================== Bug verification tests ====================

    fn make_key(code: KeyCode) -> KeyEvent {
        make_key_with_kind(code, crossterm::event::KeyEventKind::Press)
    }

    fn make_key_with_kind(code: KeyCode, kind: crossterm::event::KeyEventKind) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn test_key_release_is_ignored_but_repeat_is_processed() {
        use crossterm::event::KeyEventKind;

        let mut app = build_mock_app_with_logs(3);
        app.info_message = Some("still visible".to_string());
        app.handle_key(make_key_with_kind(KeyCode::Down, KeyEventKind::Release), 20);

        assert_eq!(app.logs_selected_index, 0);
        assert_eq!(app.info_message.as_deref(), Some("still visible"));

        app.info_message = None;
        app.handle_key(make_key_with_kind(KeyCode::Down, KeyEventKind::Repeat), 20);
        assert_eq!(app.logs_selected_index, 1);
        app.handle_key(make_key(KeyCode::Down), 20);
        assert_eq!(app.logs_selected_index, 2);
    }

    /// FIXED: `Ctrl+b` in the Explorer must page up, not trigger batch load
    /// (the `b` arm used to match before modifiers were checked)
    #[test]
    fn test_ctrl_b_pages_up_in_explorer() {
        let mut app = build_mock_app_with_explorer_files();
        app.explorer_selected_index = 2;

        let ctrl_b = KeyEvent {
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        app.handle_key(ctrl_b, 20);

        assert_eq!(
            app.screen,
            AppScreen::Explorer,
            "Ctrl+b must not trigger batch load"
        );
        assert_eq!(app.explorer_selected_index, 0, "Ctrl+b should page up");
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

    // ==================== Phase 1 security/bug fix tests ====================

    /// SEC-1: ReDoS-prone regex should be rejected by size_limit and fall back to plain text
    #[test]
    fn test_redos_regex_rejected() {
        let mut app = build_mock_app_with_diverse_logs();
        // This pattern compiles to a very large NFA; size_limit should reject it
        app.filter.text = Some("(a+)+$".repeat(100));
        app.apply_filter();
        // Should not hang — the regex fails to compile and falls back to plain text search
        // Since no log contains the literal "(a+)+$" repeated, result should be 0
        assert_eq!(app.filtered_log_indices.len(), 0);
    }

    /// SEC-5: Filter input should be bounded at 1024 characters
    #[test]
    fn test_filter_input_length_limit() {
        let mut app = build_mock_app_with_diverse_logs();
        app.screen = AppScreen::LogViewer;

        // Enter filter input mode
        app.handle_key(make_key(KeyCode::Char('/')), 20);

        // Type 1030 characters
        for _ in 0..1030 {
            app.handle_key(make_key(KeyCode::Char('x')), 20);
        }

        assert_eq!(
            app.filter_input.len(),
            1024,
            "Filter input should be capped at 1024"
        );
    }

    /// UX-4: Empty filter submission should show "Filter cleared" feedback
    #[test]
    fn test_empty_filter_shows_cleared_message() {
        let mut app = build_mock_app_with_diverse_logs();
        app.screen = AppScreen::LogViewer;

        // Enter filter input mode and submit empty
        app.handle_key(make_key(KeyCode::Char('/')), 20);
        app.handle_key(make_key(KeyCode::Enter), 20);

        assert_eq!(
            app.info_message,
            Some("Filter cleared".to_string()),
            "Empty filter submission should show feedback"
        );
    }

    #[test]
    fn test_invalid_level_preserves_active_filter() {
        let mut app = build_mock_app_with_diverse_logs();
        app.filter.min_level = Some(crate::parser::LogLevel::Warn);
        app.apply_filter();
        let filtered_before = app.filtered_log_indices.clone();
        let generation_before = app.filter_generation;
        app.logs_selected_index = 1;

        app.handle_key(make_key(KeyCode::Char('l')), 20);
        app.handle_key(make_key(KeyCode::Char('x')), 20);
        app.handle_key(make_key(KeyCode::Enter), 20);

        assert_eq!(app.filter.min_level, Some(crate::parser::LogLevel::Warn));
        assert_eq!(app.filtered_log_indices, filtered_before);
        assert_eq!(app.logs_selected_index, 1);
        assert_eq!(app.filter_generation, generation_before);
        assert_eq!(
            app.error_message.as_deref(),
            Some("Invalid level; use F/E/W/I/D/V")
        );
        assert_eq!(app.info_message, None);
        assert_eq!(app.filter_input_mode, None);
    }

    #[test]
    fn test_level_filter_accepts_names_and_empty_clears() {
        let mut app = build_mock_app_with_diverse_logs();
        let generation_before = app.filter_generation;

        app.handle_key(make_key(KeyCode::Char('l')), 20);
        for character in "ERROR".chars() {
            app.handle_key(make_key(KeyCode::Char(character)), 20);
        }
        app.handle_key(make_key(KeyCode::Enter), 20);

        assert_eq!(app.filter.min_level, Some(crate::parser::LogLevel::Error));
        assert_eq!(app.filter_generation, generation_before + 1);
        assert_eq!(app.error_message, None);

        app.handle_key(make_key(KeyCode::Char('l')), 20);
        app.handle_key(make_key(KeyCode::Enter), 20);

        assert_eq!(app.filter.min_level, None);
        assert_eq!(app.filter_generation, generation_before + 2);
        assert_eq!(app.filtered_log_indices.len(), app.logs.len());
        assert_eq!(app.info_message.as_deref(), Some("Filter cleared"));
    }

    /// BUG-2: Accessing explorer with out-of-bounds index should not panic
    #[test]
    fn test_explorer_bounds_safety() {
        let mut app = App::new();
        // Manually set index beyond items (simulates stale index after directory change error)
        app.explorer_selected_index = 999;
        app.explorer_items = vec![]; // empty

        // These should not panic
        app.handle_key(make_key(KeyCode::Enter), 20);
        app.handle_key(make_key(KeyCode::Char('b')), 20);
    }

    /// UX-1: Esc on Explorer should quit the app
    #[test]
    fn test_esc_on_explorer_quits() {
        let mut app = App::new();
        assert_eq!(app.screen, AppScreen::Explorer);
        assert!(!app.should_quit);

        app.handle_key(make_key(KeyCode::Esc), 20);
        assert!(app.should_quit, "Esc on Explorer should quit the app");
    }

    /// UX-1: Esc on LogViewer goes back to Explorer
    #[test]
    fn test_esc_on_log_viewer_goes_back() {
        let mut app = build_mock_app_with_logs(5);
        app.screen = AppScreen::LogViewer;

        app.handle_key(make_key(KeyCode::Esc), 20);
        assert_eq!(app.screen, AppScreen::Explorer);
    }

    /// BUG-1: apply_filter bumps filter_generation
    #[test]
    fn test_filter_generation_increments() {
        let mut app = build_mock_app_with_diverse_logs();
        let gen_before = app.filter_generation;

        app.apply_filter();
        assert_eq!(
            app.filter_generation,
            gen_before + 1,
            "apply_filter should increment filter_generation"
        );
    }

    #[test]
    fn test_on_tick_limits_messages_processed_per_frame() {
        let mut app = App::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let total = MAX_LOGS_PER_TICK + 5;

        for i in 0..total {
            let text = format!("message {i}");
            tx.send(LogEntry::new(
                DltMessage::new(
                    i as u64,
                    "ECU1".to_string(),
                    None,
                    None,
                    None,
                    text.into_bytes(),
                ),
                None,
                0,
            ))
            .unwrap();
        }
        drop(tx);
        app.log_receiver = Some(rx);
        app.is_loading = true;
        app.auto_scroll = true;

        app.on_tick();

        assert_eq!(app.logs.len(), MAX_LOGS_PER_TICK);
        assert_eq!(app.filtered_log_indices.len(), MAX_LOGS_PER_TICK);
        assert_eq!(app.source_counts().get("-"), Some(&MAX_LOGS_PER_TICK));
        assert_eq!(app.ctx_missing_count, MAX_LOGS_PER_TICK);
        assert_eq!(app.logs_selected_index, MAX_LOGS_PER_TICK - 1);
        assert!(app.log_receiver.is_some());
        assert!(app.is_loading);

        app.on_tick();

        assert_eq!(app.logs.len(), total);
        assert_eq!(app.filtered_log_indices.len(), total);
        assert_eq!(app.source_counts().get("-"), Some(&total));
        assert_eq!(app.ctx_missing_count, total);
        assert_eq!(app.logs_selected_index, total - 1);
        assert!(app.log_receiver.is_none());
        assert!(!app.is_loading);
    }

    #[test]
    fn test_filter_scan_processes_large_logs_incrementally() {
        let mut app = App::new();
        for i in 0..(MAX_LOGS_PER_TICK * 2 + 1) {
            app.logs.push(mock_entry(i as u64, "message", None, 0));
        }

        app.begin_filter();
        assert!(app.filtered_log_indices.is_empty());

        app.on_tick();
        assert_eq!(app.filtered_log_indices.len(), MAX_LOGS_PER_TICK);
        assert!(app.filter_scan_cursor.is_some());

        app.on_tick();
        assert_eq!(app.filtered_log_indices.len(), MAX_LOGS_PER_TICK * 2);
        assert!(app.filter_scan_cursor.is_some());

        app.on_tick();
        assert_eq!(app.filtered_log_indices.len(), MAX_LOGS_PER_TICK * 2 + 1);
        assert!(app.filter_scan_cursor.is_none());
    }

    #[test]
    fn test_export_writes_logs_incrementally_and_finishes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("export.txt");
        let mut app = App::new();
        for i in 0..(MAX_LOGS_PER_TICK + 1) {
            app.logs.push(mock_entry(i as u64, "exported", None, 0));
        }
        let indices: Vec<usize> = (0..app.logs.len()).collect();
        let writer = crate::exporter::TextWriter::create_new(&path).unwrap();
        app.export_job = Some(ExportJob {
            path: path.clone(),
            indices,
            cursor: 0,
            writer: Some(writer),
            completed: false,
        });

        app.on_tick();
        assert_eq!(app.export_job.as_ref().unwrap().cursor, MAX_LOGS_PER_TICK);
        assert!(!app.info_message.as_deref().unwrap().starts_with("Exported"));

        app.on_tick();
        assert!(app.export_job.is_none());
        assert!(app.info_message.as_deref().unwrap().starts_with("Exported"));
        assert_eq!(
            std::fs::read_to_string(path).unwrap().lines().count(),
            MAX_LOGS_PER_TICK + 2
        );
    }

    #[test]
    fn test_export_rejects_incomplete_filter_results() {
        let mut app = App::new();
        app.filtered_log_indices = vec![0];
        app.filter_scan_cursor = Some(1);

        app.on_export();

        assert!(app.export_job.is_none());
        assert_eq!(
            app.error_message.as_deref(),
            Some("Wait for filtering to finish before exporting")
        );
    }

    #[test]
    fn test_file_load_errors_are_reported_when_loading_finishes() {
        let mut app = App::new();
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        app.log_receiver = Some(rx);
        app.is_loading = true;
        app.file_errors = Some(Arc::new(Mutex::new(vec![
            "broken.dlt.gz: invalid gzip header".to_string(),
        ])));

        app.on_tick();

        assert_eq!(
            app.error_message.as_deref(),
            Some("Failed to load broken.dlt.gz: invalid gzip header")
        );
        assert!(app.file_errors.is_none());
        assert!(!app.is_loading);
    }

    #[test]
    fn test_missing_file_error_reaches_the_ui() {
        let temp_dir = tempfile::tempdir().unwrap();
        let missing_path = temp_dir.path().join("missing.dlt");
        let mut app = App::new();

        app.load_files(vec![missing_path]).unwrap();
        for _ in 0..100 {
            app.on_tick();
            if !app.is_loading {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        assert!(!app.is_loading);
        let error = app.error_message.as_deref().unwrap_or_default();
        assert!(error.starts_with("Failed to load "));
        assert!(error.contains("missing.dlt"));
    }

    #[test]
    fn test_file_load_resets_tcp_state() {
        let mut app = App::new();
        app.connection_info = Some("localhost:3490".to_string());
        app.auto_scroll = true;
        app.tcp_error = Some(Arc::new(Mutex::new(Some("old error".to_string()))));

        app.load_files(Vec::new()).unwrap();

        assert!(app.connection_info.is_none());
        assert!(!app.auto_scroll);
        assert!(app.tcp_error.is_none());
    }
}
