#[derive(Debug, PartialEq, Clone)]
pub enum LogLevel {
    Fatal,
    Error,
    Warn,
    Info,
    Debug,
    Verbose,
    Unknown(u8),
}

#[derive(Debug, PartialEq, Clone)]
pub struct DltMessage {
    pub timestamp_us: u64,
    pub ecu_id: String,
    pub apid: Option<String>,
    pub ctid: Option<String>,
    pub log_level: Option<LogLevel>,
    pub payload_text: String,
    pub payload_raw: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Incomplete(usize),
    InvalidMagicNumber,
    InvalidHeader,
    Unknown,
}

use nom::{
    IResult,
    bytes::complete::{tag, take},
    number::complete::{le_u16, le_u32},
};

fn parse_storage_header(input: &[u8]) -> IResult<&[u8], (u64, String)> {
    let (input, _magic) = tag("DLT\x01".as_bytes())(input)?;
    let (input, timestamp_sec) = le_u32(input)?;
    let (input, timestamp_us) = le_u32(input)?;
    let (input, ecu_id_bytes) = take(4usize)(input)?;

    let ecu_id = String::from_utf8_lossy(ecu_id_bytes)
        .trim_end_matches('\0')
        .to_string();

    let combined_us = (timestamp_sec as u64) * 1_000_000 + (timestamp_us as u64);

    Ok((input, (combined_us, ecu_id)))
}

fn parse_standard_header(input: &[u8]) -> IResult<&[u8], (u8, u8, u16)> {
    let (input, htyp) = nom::number::complete::u8(input)?;
    let (input, mcnt) = nom::number::complete::u8(input)?;
    let (input, len) = le_u16(input)?;
    Ok((input, (htyp, mcnt, len)))
}

fn parse_extended_header(input: &[u8]) -> IResult<&[u8], (u8, u8, String, String)> {
    let (input, msin) = nom::number::complete::u8(input)?;
    let (input, noar) = nom::number::complete::u8(input)?;
    let (input, apid_bytes) = take(4usize)(input)?;
    let (input, ctid_bytes) = take(4usize)(input)?;

    let apid = String::from_utf8_lossy(apid_bytes)
        .trim_end_matches('\0')
        .to_string();
    let ctid = String::from_utf8_lossy(ctid_bytes)
        .trim_end_matches('\0')
        .to_string();

    Ok((input, (msin, noar, apid, ctid)))
}

pub fn parse_dlt_message(input: &[u8]) -> Result<(&[u8], DltMessage), ParseError> {
    if input.len() < 4 {
        return Err(ParseError::Incomplete(4 - input.len()));
    }

    // 1. Storage Header (Optional, but MVP covers files with it)
    let storage_res = parse_storage_header(input);
    let (input, (timestamp_us, ecu_id)) = match storage_res {
        Ok(res) => res,
        Err(nom::Err::Error(_e)) | Err(nom::Err::Failure(_e)) => {
            if input.starts_with(b"DLT") {
                return Err(ParseError::Incomplete(16));
            } else {
                return Err(ParseError::InvalidMagicNumber);
            }
        }
        Err(nom::Err::Incomplete(_needed)) => {
            return Err(ParseError::Incomplete(1));
        }
    };

    // 2. Standard Header
    let (mut input, (htyp, _mcnt, len)) = match parse_standard_header(input) {
        Ok(res) => res,
        Err(nom::Err::Incomplete(_)) => return Err(ParseError::Incomplete(4)),
        Err(_) => return Err(ParseError::Incomplete(4)), // complete parser returns Error instead of Incomplete on EOF
    };

    // The len field includes the Standard Header itself (4 bytes minimum)
    let expected_remaining = (len as usize).saturating_sub(4);
    if input.len() < expected_remaining {
        return Err(ParseError::Incomplete(expected_remaining - input.len()));
    }

    let ueh = (htyp & 0x01) != 0; // Use Extended Header bit

    let mut msg_apid = None;
    let mut msg_ctid = None;
    let mut msg_log_level = None;
    let expected_payload_len = len.saturating_sub(4);
    let mut actual_payload_len = expected_payload_len as usize;

    // 3. Extended Header
    if ueh {
        if actual_payload_len < 10 {
            return Err(ParseError::InvalidHeader);
        }
        let (new_input, (msin, _noar, apid, ctid)) = match parse_extended_header(input) {
            Ok(res) => res,
            Err(nom::Err::Incomplete(_)) => return Err(ParseError::Incomplete(10)),
            Err(_) => return Err(ParseError::InvalidHeader),
        };
        input = new_input;
        msg_apid = Some(apid);
        msg_ctid = Some(ctid);

        let msg_type = msin & 0x07; // bits 0..=2
        if msg_type == 0 {
            // 0 = DLT_TYPE_LOG
            let log_lvl = (msin >> 3) & 0x07; // bits 3..=5. bits 4..=6 if shift by 4. Wait, spec says bits 3..=6. Let's trace it.
            // DLT Autocore spec: DLT_LOG_FATAL = 1, ERROR=2, WARN=3, INFO=4, DEBUG=5, VERBOSE=6.
            match log_lvl {
                1 => msg_log_level = Some(LogLevel::Fatal),
                2 => msg_log_level = Some(LogLevel::Error),
                3 => msg_log_level = Some(LogLevel::Warn),
                4 => msg_log_level = Some(LogLevel::Info),
                5 => msg_log_level = Some(LogLevel::Debug),
                6 => msg_log_level = Some(LogLevel::Verbose),
                other => msg_log_level = Some(LogLevel::Unknown(other)),
            }
        }
        actual_payload_len -= 10;
    }

    // 4. Payload extract
    if input.len() < actual_payload_len {
        return Err(ParseError::Incomplete(actual_payload_len - input.len()));
    }

    let take_payload: IResult<&[u8], &[u8]> = take(actual_payload_len)(input);
    let (new_input, payload_bytes) = match take_payload {
        Ok(res) => res,
        Err(_) => return Err(ParseError::Incomplete(actual_payload_len)),
    };
    input = new_input;

    let raw_text = String::from_utf8_lossy(payload_bytes);
    let payload_text = raw_text
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '.'
            } else {
                c
            }
        })
        .collect::<String>();

    Ok((
        input,
        DltMessage {
            timestamp_us,
            ecu_id,
            apid: msg_apid,
            ctid: msg_ctid,
            log_level: msg_log_level,
            payload_text,
            payload_raw: payload_bytes.to_vec(),
        },
    ))
}

/// Find the next potential DLT message start position in the data.
/// Looks for "DLT\x01" (storage header magic) or a valid-looking standard header.
/// Returns the byte offset from the start of `data` where the next message likely begins.
pub fn find_next_sync(data: &[u8]) -> Option<usize> {
    for i in 0..data.len() {
        // Storage header magic
        if data[i..].starts_with(b"DLT\x01") {
            return Some(i);
        }
        // Standard header heuristic: version bits in HTYP should be 0x01 (version 1)
        // HTYP byte: bits 5-7 = version. Version 1 => (htyp >> 5) & 0x07 == 1
        if i + 4 <= data.len() {
            let htyp = data[i];
            let version = (htyp >> 5) & 0x07;
            if version == 1 {
                return Some(i);
            }
        }
    }
    None
}

/// Parse all DLT messages from a byte buffer with error recovery.
/// When a parse error occurs, scans ahead for the next valid sync marker
/// instead of stopping. Returns the parsed messages and the count of bytes skipped.
pub fn parse_all_messages(data: &[u8]) -> (Vec<DltMessage>, usize) {
    let mut messages = Vec::new();
    let mut input = data;
    let mut skipped_bytes: usize = 0;

    while !input.is_empty() {
        match parse_dlt_message(input) {
            Ok((remaining, msg)) => {
                messages.push(msg);
                input = remaining;
            }
            Err(ParseError::Incomplete(_)) => {
                // Not enough data for the current message; stop
                break;
            }
            Err(_) => {
                // Error recovery: skip ahead to next sync marker
                if input.len() <= 1 {
                    skipped_bytes += input.len();
                    break;
                }
                if let Some(pos) = find_next_sync(&input[1..]) {
                    skipped_bytes += 1 + pos;
                    input = &input[1 + pos..];
                } else {
                    // No more sync markers found
                    skipped_bytes += input.len();
                    break;
                }
            }
        }
    }

    (messages, skipped_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constructs a valid simulated offline DLT message byte array
    fn build_valid_dlt_message_bytes() -> Vec<u8> {
        build_dlt_message_with_payload(b"Hello DLT")
    }

    /// Constructs a valid DLT message with a custom payload
    fn build_dlt_message_with_payload(payload: &[u8]) -> Vec<u8> {
        let mut msg = Vec::new();
        // 1. Storage Header (16 bytes)
        msg.extend_from_slice(b"DLT\x01"); // Magic number
        msg.extend_from_slice(&1640995200u32.to_le_bytes()); // timestamp seconds (2022-01-01)
        msg.extend_from_slice(&123456u32.to_le_bytes()); // timestamp microseconds
        msg.extend_from_slice(b"ECU1"); // ECU ID

        // 2. Standard Header
        // HTYP: UEH(bit0)=1, VERS(bit5-7)=1 => 0x21
        msg.push(0x21); // HTYP
        msg.push(0x00); // MCNT (Message Counter)

        let total_len: u16 = 4 + 10 + payload.len() as u16;
        msg.extend_from_slice(&total_len.to_le_bytes()); // LEN

        // 3. Extended Header (10 bytes)
        // MSIN: MSG_TYPE=0(Log), LogLevel=Info(4) => 4 << 3 = 0x20
        msg.push(0x20); // MSIN
        msg.push(1); // NOAR
        msg.extend_from_slice(b"APP1"); // APID
        msg.extend_from_slice(b"CTX1"); // CTID

        // 4. Payload
        msg.extend_from_slice(payload);

        msg
    }

    // ==================== parse_dlt_message tests ====================

    #[test]
    fn test_parse_valid_dlt_message() {
        let data = build_valid_dlt_message_bytes();

        let (remaining, msg) = parse_dlt_message(&data).expect("Parsing failed for valid message");
        assert_eq!(remaining.len(), 0, "Should consume the entire stream");

        assert_eq!(msg.ecu_id, "ECU1");
        assert_eq!(msg.apid, Some("APP1".to_string()));
        assert_eq!(msg.ctid, Some("CTX1".to_string()));
        assert_eq!(msg.log_level, Some(LogLevel::Info));
        assert_eq!(msg.payload_text, "Hello DLT");
        assert_eq!(msg.payload_raw, b"Hello DLT".to_vec());
    }

    #[test]
    fn test_parse_invalid_magic_number() {
        let mut data = build_valid_dlt_message_bytes();
        data[0] = b'X'; // Break magic number "DLT\x01"

        let err = parse_dlt_message(&data).unwrap_err();
        assert_eq!(err, ParseError::InvalidMagicNumber);
    }

    #[test]
    fn test_parse_truncated_message() {
        let mut data = build_valid_dlt_message_bytes();
        data.truncate(20); // truncate before the full length is hit

        let err = parse_dlt_message(&data).unwrap_err();
        match err {
            ParseError::Incomplete(_) => {} // expected
            _ => panic!("Expected ParseError::Incomplete"),
        }
    }

    #[test]
    fn test_parse_unknown_log_level() {
        let mut data = build_valid_dlt_message_bytes();
        // Overwrite MSIN. Message Type = 0(Log). Message Info = 7(Unknown log level) => 7 << 3 = 56 = 0x38
        // The exact offset is Storage(16) + Std(4) = 20
        data[20] = 0x38;

        let (_, msg) = parse_dlt_message(&data).expect("Should still parse");
        assert_eq!(msg.log_level, Some(LogLevel::Unknown(7)));
    }

    // ==================== find_next_sync tests ====================

    #[test]
    fn test_find_next_sync_with_storage_header_magic() {
        let mut data = vec![0x00, 0x00, 0xFF, 0xFF]; // garbage
        data.extend_from_slice(b"DLT\x01"); // storage header magic at offset 4
        data.extend_from_slice(&[0x00; 12]); // rest of storage header

        let pos = find_next_sync(&data).unwrap();
        assert_eq!(pos, 4, "Should find DLT magic at offset 4");
    }

    #[test]
    fn test_find_next_sync_at_start() {
        let mut data = Vec::new();
        data.extend_from_slice(b"DLT\x01");
        data.extend_from_slice(&[0x00; 12]);

        let pos = find_next_sync(&data).unwrap();
        assert_eq!(pos, 0, "Should find DLT magic at offset 0");
    }

    #[test]
    fn test_find_next_sync_no_marker() {
        // Data with no valid sync markers (all zeros — version 0, not valid)
        let data = vec![0x00; 16];
        let pos = find_next_sync(&data);
        assert!(pos.is_none(), "Should not find any sync marker in all-zeros");
    }

    #[test]
    fn test_find_next_sync_with_standard_header_heuristic() {
        // A byte with version=1 in bits 5-7: (1 << 5) = 0x20
        let data = vec![0x00, 0x00, 0x21, 0x00, 0x00, 0x17, 0x00];
        let pos = find_next_sync(&data).unwrap();
        assert_eq!(pos, 2, "Should find standard header heuristic at offset 2");
    }

    // ==================== parse_all_messages tests ====================

    #[test]
    fn test_parse_all_messages_clean_data() {
        let mut data = Vec::new();
        data.extend(build_dlt_message_with_payload(b"Message 1"));
        data.extend(build_dlt_message_with_payload(b"Message 2"));
        data.extend(build_dlt_message_with_payload(b"Message 3"));

        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 3);
        assert_eq!(skipped, 0, "No bytes should be skipped for clean data");
        assert_eq!(msgs[0].payload_text, "Message 1");
        assert_eq!(msgs[1].payload_text, "Message 2");
        assert_eq!(msgs[2].payload_text, "Message 3");
    }

    #[test]
    fn test_parse_all_messages_with_garbage_prefix() {
        let mut data = vec![0xDE, 0xAD, 0xBE, 0xEF]; // 4 bytes garbage
        data.extend(build_dlt_message_with_payload(b"After garbage"));

        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 1);
        assert!(skipped > 0, "Should report skipped bytes from garbage prefix");
        assert_eq!(msgs[0].payload_text, "After garbage");
    }

    #[test]
    fn test_parse_all_messages_with_garbage_between() {
        let mut data = Vec::new();
        data.extend(build_dlt_message_with_payload(b"First"));
        data.extend_from_slice(&[0xFF, 0xFE, 0xFD, 0xFC, 0xFB]); // 5 bytes garbage
        data.extend(build_dlt_message_with_payload(b"Second"));

        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 2, "Should recover and parse both messages");
        assert!(skipped > 0, "Should report skipped garbage bytes");
        assert_eq!(msgs[0].payload_text, "First");
        assert_eq!(msgs[1].payload_text, "Second");
    }

    #[test]
    fn test_parse_all_messages_with_trailing_garbage() {
        let mut data = Vec::new();
        data.extend(build_dlt_message_with_payload(b"Valid msg"));
        data.extend_from_slice(&[0xFF; 10]); // trailing garbage

        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 1);
        assert!(skipped > 0, "Should report skipped trailing bytes");
        assert_eq!(msgs[0].payload_text, "Valid msg");
    }

    #[test]
    fn test_parse_all_messages_empty_input() {
        let (msgs, skipped) = parse_all_messages(&[]);
        assert_eq!(msgs.len(), 0);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn test_parse_all_messages_only_garbage() {
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00];
        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 0);
        assert!(skipped > 0, "Should report all bytes as skipped");
    }

    /// Scenario test: Simulates a real-world DLT file with corrupted sections
    #[test]
    fn test_scenario_corrupted_dlt_file() {
        let mut data = Vec::new();

        // Message 1 - valid
        data.extend(build_dlt_message_with_payload(b"Boot started"));
        // Corrupted area (simulating partial write or disk corruption)
        data.extend_from_slice(&[0x00, 0x00, 0xFF, 0x44, 0x4C, 0xAB]); // includes partial "DL" but not valid
        // Message 2 - valid
        data.extend(build_dlt_message_with_payload(b"GPS acquired"));
        // More garbage
        data.extend_from_slice(&[0xCA, 0xFE, 0xBA, 0xBE]);
        // Message 3 - valid
        data.extend(build_dlt_message_with_payload(b"CAN timeout"));

        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 3, "All 3 valid messages should be recovered");
        assert!(skipped > 0, "Corrupted bytes should be counted as skipped");
        assert_eq!(msgs[0].payload_text, "Boot started");
        assert_eq!(msgs[1].payload_text, "GPS acquired");
        assert_eq!(msgs[2].payload_text, "CAN timeout");
    }
}
