use crate::parser::DltMessage;
use crate::ui::format_timestamp;
use std::fs::File;
use std::io::{self, Write};

/// Exports a slice of DltMessage references to a text file.
pub fn export_to_txt(logs: &[&DltMessage], path: &str) -> io::Result<()> {
    let mut file = File::create(path)?;

    // Write a simple header
    writeln!(file, "Timestamp, ECU, APP, CTX, Level, Payload")?;

    for log in logs {
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
            file,
            "{} [{}] [{}] [{}] [{}] {}",
            format_timestamp(log.timestamp_us),
            log.ecu_id,
            log.apid.as_deref().unwrap_or("-"),
            log.ctid.as_deref().unwrap_or("-"),
            level_str,
            log.payload_text
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::NamedTempFile;

    #[test]
    fn test_export_to_txt() {
        let msg = DltMessage {
            timestamp_us: 1_234_567_890,
            ecu_id: "ECU1".to_string(),
            apid: Some("APP1".to_string()),
            ctid: Some("CTX1".to_string()),
            log_level: Some(crate::parser::LogLevel::Info),
            payload_text: "test export message".to_string(),
            payload_raw: b"test export message".to_vec(),
        };

        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_str().unwrap();

        let logs = vec![&msg];
        export_to_txt(&logs, path).unwrap();

        let mut file = File::open(path).unwrap();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();

        assert!(content.contains("Timestamp, ECU, APP, CTX, Level, Payload"));
        assert!(content.contains("[ECU1] [APP1] [CTX1] [INF] test export message"));
        assert!(content.contains("00:20:34.567890")); // format_timestamp result
    }
}
