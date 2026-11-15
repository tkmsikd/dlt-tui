use crate::parser::DltMessage;
use crate::ui::format_timestamp;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;

pub struct TextWriter {
    file: BufWriter<File>,
}

impl TextWriter {
    pub fn create_new(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut writer = Self {
            file: BufWriter::new(file),
        };
        if let Err(error) = writeln!(writer.file, "Timestamp, ECU, APP, CTX, Level, Payload") {
            drop(writer);
            let _ = std::fs::remove_file(path);
            return Err(error);
        }
        Ok(writer)
    }

    pub fn write_message(&mut self, log: &DltMessage) -> io::Result<()> {
        let level_str = match &log.log_level {
            Some(crate::parser::LogLevel::Fatal) => "FTL",
            Some(crate::parser::LogLevel::Error) => "ERR",
            Some(crate::parser::LogLevel::Warn) => "WRN",
            Some(crate::parser::LogLevel::Info) => "INF",
            Some(crate::parser::LogLevel::Debug) => "DBG",
            Some(crate::parser::LogLevel::Verbose) => "VRB",
            Some(crate::parser::LogLevel::Unknown(_)) => "UNK",
            None => "---",
        };

        writeln!(
            self.file,
            "{} [{}] [{}] [{}] [{}] {}",
            format_timestamp(log.timestamp_us),
            log.ecu_id,
            log.apid.as_deref().unwrap_or("-"),
            log.ctid.as_deref().unwrap_or("-"),
            level_str,
            log.payload_text()
        )
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Exports a slice of DltMessage references to a text file.
pub fn export_to_txt(logs: &[&DltMessage], path: &str) -> io::Result<()> {
    let mut writer = TextWriter::create_new(Path::new(path))?;
    for log in logs {
        writer.write_message(log)?;
    }
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Read};
    use tempfile::tempdir;

    #[test]
    fn test_export_to_txt() {
        let msg = DltMessage::new(
            1_234_567_890,
            "ECU1".to_string(),
            Some("APP1".to_string()),
            Some("CTX1".to_string()),
            Some(crate::parser::LogLevel::Info),
            b"test export message".to_vec(),
        );

        let temp_dir = tempdir().unwrap();
        let export_path = temp_dir.path().join("export.txt");
        let path = export_path.to_str().unwrap();

        let logs = vec![&msg];
        assert!(!msg.payload_text_is_initialized());
        export_to_txt(&logs, path).unwrap();
        assert!(msg.payload_text_is_initialized());

        let mut file = std::fs::File::open(path).unwrap();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();

        assert!(content.contains("Timestamp, ECU, APP, CTX, Level, Payload"));
        assert!(content.contains("[ECU1] [APP1] [CTX1] [INF] test export message"));
        assert!(content.contains("00:20:34.567890")); // format_timestamp result
    }

    #[test]
    fn test_export_does_not_overwrite_existing_file() {
        let temp_dir = tempdir().unwrap();
        let export_path = temp_dir.path().join("existing.txt");
        fs::write(&export_path, "keep me").unwrap();

        let error = export_to_txt(&[], export_path.to_str().unwrap()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(export_path).unwrap(), "keep me");
    }
}
