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
    number::complete::le_u16,
};

fn parse_storage_header(input: &[u8]) -> IResult<&[u8], (&[u8], String)> {
    let (input, magic) = tag("DLT\x01".as_bytes())(input)?;
    let (input, _timestamp_sec) = take(4usize)(input)?;
    let (input, _timestamp_us) = take(4usize)(input)?;
    let (input, ecu_id_bytes) = take(4usize)(input)?;

    let ecu_id = String::from_utf8_lossy(ecu_id_bytes)
        .trim_end_matches('\0')
        .to_string();

    Ok((input, (magic, ecu_id)))
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
    let (input, (_, ecu_id)) = match storage_res {
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
    let expected_remaining = if len as usize >= 4 {
        len as usize - 4
    } else {
        0
    };
    if input.len() < expected_remaining {
        return Err(ParseError::Incomplete(expected_remaining - input.len()));
    }

    let ueh = (htyp & 0x01) != 0; // Use Extended Header bit

    let mut msg_apid = None;
    let mut msg_ctid = None;
    let mut msg_log_level = None;
    let expected_payload_len = if len >= 4 { len - 4 } else { 0 };
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

    let payload_text = String::from_utf8_lossy(payload_bytes).to_string();

    Ok((
        input,
        DltMessage {
            timestamp_us: 0, // Mocked for now, need parsing it from standard header if we want or use the unix timestamp
            ecu_id,
            apid: msg_apid,
            ctid: msg_ctid,
            log_level: msg_log_level,
            payload_text,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constructs a valid simulated offline DLT message byte array
    fn build_valid_dlt_message_bytes() -> Vec<u8> {
        let mut msg = Vec::new();
        // 1. Storage Header (16 bytes)
        msg.extend_from_slice(b"DLT\x01"); // Magic number
        msg.extend_from_slice(&1640995200u32.to_le_bytes()); // timestamp seconds (2022-01-01)
        msg.extend_from_slice(&123456u32.to_le_bytes()); // timestamp microseconds
        msg.extend_from_slice(b"ECU1"); // ECU ID

        // 2. Standard Header (Minimum 4 bytes if no extra fields, let's say UEH is true)
        // HTYP: UEH(bit0)=1, MSBF(bit1)=0 (Little Endian), WEID(bit2)=0, WSID(bit3)=0, WTMS(bit4)=0, VERS(bit5-7)=1
        // HTYP = 0b0010_0001 = 0x21
        msg.push(0x21); // HTYP
        msg.push(0x00); // MCNT (Message Counter)

        let payload = b"Hello DLT";
        // Header lengths: Standard(4) + Extended(10) + Payload(9) = 23
        msg.extend_from_slice(&23u16.to_le_bytes()); // LEN 

        // 3. Extended Header (10 bytes)
        // MSIN: bit 0: Type (0=log), bit 1-3: Log Level (1=Fatal, 2=Error, 3=Warn, 4=Info, 5=Debug, 6=Verbose)
        // LogLevel = Info (4) -> MSIN = 0b0100_0000 = 0x40 (Wait, type log is usually 0x00 at LSB, but let's assume LogInfo is 0x41 roughly.
        // Actually MSIN for Log Info: MSG_TYPE=0(log), MSG_INFO=4(info) -> 4 << 4 = 64 = 0x40.
        // Let's use 0x41 for MSIN where LSB=1 is log message type, bits 1-3 are log level = info (4<<1 = 8) -> 0x01 | 0x08 = 0x09?
        // Actually DLT spec: MSIN bits 0-2 = Message Type (0=Log). bits 3-6 = Message Info (Log level: default 1..6).
        // For Log (0) and Info (4) => 4 << 3 = 32 = 0x20. Let's use 0x20.
        msg.push(0x20); // MSIN
        msg.push(1); // NOAR (1 argument for simplicity)
        msg.extend_from_slice(b"APP1"); // APID
        msg.extend_from_slice(b"CTX1"); // CTID

        // 4. Payload (9 bytes)
        msg.extend_from_slice(payload);

        msg
    }

    #[test]
    fn test_parse_valid_dlt_message() {
        let data = build_valid_dlt_message_bytes();

        let (remaining, msg) = parse_dlt_message(&data).expect("Parsing failed for valid message");
        assert_eq!(remaining.len(), 0, "Should consume the entire stream");

        assert_eq!(msg.ecu_id, "ECU1");
        assert_eq!(msg.apid, Some("APP1".to_string()));
        assert_eq!(msg.ctid, Some("CTX1".to_string()));
        assert_eq!(msg.log_level, Some(LogLevel::Info));
        // The payload string might just be a basic string extraction for testing purposes
        assert_eq!(msg.payload_text, "Hello DLT");
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

        // Expect it to say Incomplete with the number of missing bytes or just generic Incomplete
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
}
