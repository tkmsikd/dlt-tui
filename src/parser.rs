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
    number::complete::{be_u16, le_u32},
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

/// Standard Header: HTYP(1) + MCNT(1) + LEN(2, always big-endian per DLT spec)
fn parse_standard_header(input: &[u8]) -> IResult<&[u8], (u8, u8, u16)> {
    let (input, htyp) = nom::number::complete::u8(input)?;
    let (input, mcnt) = nom::number::complete::u8(input)?;
    let (input, len) = be_u16(input)?;
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

fn read_4byte_id(input: &[u8]) -> IResult<&[u8], String> {
    let (input, bytes) = take(4usize)(input)?;
    Ok((
        input,
        String::from_utf8_lossy(bytes)
            .trim_end_matches('\0')
            .to_string(),
    ))
}

/// Decode a DLT verbose-mode payload into a human-readable string.
/// Each argument is encoded as TypeInfo(4 bytes) + optional length + data.
/// `msbf` indicates the byte order of the payload content.
fn decode_verbose_payload(payload: &[u8], noar: u8, msbf: bool) -> String {
    let mut parts = Vec::new();
    let mut pos = 0;

    for _ in 0..noar {
        if pos + 4 > payload.len() {
            break;
        }

        let type_info = if msbf {
            u32::from_be_bytes([
                payload[pos],
                payload[pos + 1],
                payload[pos + 2],
                payload[pos + 3],
            ])
        } else {
            u32::from_le_bytes([
                payload[pos],
                payload[pos + 1],
                payload[pos + 2],
                payload[pos + 3],
            ])
        };
        pos += 4;

        // TypeInfo bit fields (AUTOSAR DLT PRS):
        // Bits 0-3: TYLE (type length)
        // Bit 4: BOOL
        // Bit 5: SINT
        // Bit 6: UINT
        // Bit 7: FLOA
        // Bit 8: APTS (array)
        // Bit 9: STRG
        // Bit 10: RAWD
        // Bit 11: VARI (variable info)
        // Bit 15: FIXP (fixed point)
        let _tyle = type_info & 0x0F;
        let is_bool = (type_info >> 4) & 1 == 1;
        let is_sint = (type_info >> 5) & 1 == 1;
        let is_uint = (type_info >> 6) & 1 == 1;
        let is_float = (type_info >> 7) & 1 == 1;
        let is_strg = (type_info >> 9) & 1 == 1;
        let is_rawd = (type_info >> 10) & 1 == 1;
        let is_vari = (type_info >> 11) & 1 == 1;

        // Handle variable info (name + unit) - skip it for display
        if is_vari {
            // Variable info has: name_length(2) + name + unit_length(2) + unit
            if pos + 2 > payload.len() {
                break;
            }
            let name_len = if msbf {
                u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize
            } else {
                u16::from_le_bytes([payload[pos], payload[pos + 1]]) as usize
            };
            pos += 2;
            pos += name_len; // skip name
            if is_strg || is_rawd {
                // no unit for string/raw
            } else {
                if pos + 2 > payload.len() {
                    break;
                }
                let unit_len = if msbf {
                    u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize
                } else {
                    u16::from_le_bytes([payload[pos], payload[pos + 1]]) as usize
                };
                pos += 2;
                pos += unit_len; // skip unit
            }
        }

        if is_strg {
            // String: length(2 bytes) + data (including null terminator)
            if pos + 2 > payload.len() {
                break;
            }
            let str_len = if msbf {
                u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize
            } else {
                u16::from_le_bytes([payload[pos], payload[pos + 1]]) as usize
            };
            pos += 2;
            if pos + str_len > payload.len() {
                // Partial string - take what we can
                let s = String::from_utf8_lossy(&payload[pos..])
                    .trim_end_matches('\0')
                    .to_string();
                parts.push(s);
                break;
            }
            let s = String::from_utf8_lossy(&payload[pos..pos + str_len])
                .trim_end_matches('\0')
                .to_string();
            parts.push(s);
            pos += str_len;
        } else if is_bool {
            if pos + 1 > payload.len() {
                break;
            }
            parts.push(if payload[pos] != 0 {
                "true".to_string()
            } else {
                "false".to_string()
            });
            pos += 1;
        } else if is_uint {
            let byte_len = match _tyle {
                1 => 1,
                2 => 2,
                3 => 4,
                4 => 8,
                5 => 16,
                _ => 4,
            };
            if pos + byte_len > payload.len() {
                break;
            }
            let val = match byte_len {
                1 => payload[pos] as u64,
                2 => {
                    if msbf {
                        u16::from_be_bytes([payload[pos], payload[pos + 1]]) as u64
                    } else {
                        u16::from_le_bytes([payload[pos], payload[pos + 1]]) as u64
                    }
                }
                4 => {
                    if msbf {
                        u32::from_be_bytes([
                            payload[pos],
                            payload[pos + 1],
                            payload[pos + 2],
                            payload[pos + 3],
                        ]) as u64
                    } else {
                        u32::from_le_bytes([
                            payload[pos],
                            payload[pos + 1],
                            payload[pos + 2],
                            payload[pos + 3],
                        ]) as u64
                    }
                }
                8 => {
                    if msbf {
                        u64::from_be_bytes([
                            payload[pos],
                            payload[pos + 1],
                            payload[pos + 2],
                            payload[pos + 3],
                            payload[pos + 4],
                            payload[pos + 5],
                            payload[pos + 6],
                            payload[pos + 7],
                        ])
                    } else {
                        u64::from_le_bytes([
                            payload[pos],
                            payload[pos + 1],
                            payload[pos + 2],
                            payload[pos + 3],
                            payload[pos + 4],
                            payload[pos + 5],
                            payload[pos + 6],
                            payload[pos + 7],
                        ])
                    }
                }
                _ => {
                    pos += byte_len;
                    parts.push(format!("<uint{}>", byte_len * 8));
                    continue;
                }
            };
            parts.push(val.to_string());
            pos += byte_len;
        } else if is_sint {
            let byte_len = match _tyle {
                1 => 1,
                2 => 2,
                3 => 4,
                4 => 8,
                _ => 4,
            };
            if pos + byte_len > payload.len() {
                break;
            }
            let val = match byte_len {
                1 => payload[pos] as i8 as i64,
                2 => {
                    if msbf {
                        i16::from_be_bytes([payload[pos], payload[pos + 1]]) as i64
                    } else {
                        i16::from_le_bytes([payload[pos], payload[pos + 1]]) as i64
                    }
                }
                4 => {
                    if msbf {
                        i32::from_be_bytes([
                            payload[pos],
                            payload[pos + 1],
                            payload[pos + 2],
                            payload[pos + 3],
                        ]) as i64
                    } else {
                        i32::from_le_bytes([
                            payload[pos],
                            payload[pos + 1],
                            payload[pos + 2],
                            payload[pos + 3],
                        ]) as i64
                    }
                }
                8 => {
                    if msbf {
                        i64::from_be_bytes([
                            payload[pos],
                            payload[pos + 1],
                            payload[pos + 2],
                            payload[pos + 3],
                            payload[pos + 4],
                            payload[pos + 5],
                            payload[pos + 6],
                            payload[pos + 7],
                        ])
                    } else {
                        i64::from_le_bytes([
                            payload[pos],
                            payload[pos + 1],
                            payload[pos + 2],
                            payload[pos + 3],
                            payload[pos + 4],
                            payload[pos + 5],
                            payload[pos + 6],
                            payload[pos + 7],
                        ])
                    }
                }
                _ => {
                    pos += byte_len;
                    parts.push(format!("<sint{}>", byte_len * 8));
                    continue;
                }
            };
            parts.push(val.to_string());
            pos += byte_len;
        } else if is_float {
            let byte_len = match _tyle {
                3 => 4, // float32
                4 => 8, // float64
                _ => 4,
            };
            if pos + byte_len > payload.len() {
                break;
            }
            if byte_len == 4 {
                let val = if msbf {
                    f32::from_be_bytes([
                        payload[pos],
                        payload[pos + 1],
                        payload[pos + 2],
                        payload[pos + 3],
                    ])
                } else {
                    f32::from_le_bytes([
                        payload[pos],
                        payload[pos + 1],
                        payload[pos + 2],
                        payload[pos + 3],
                    ])
                };
                parts.push(format!("{:.6}", val));
            } else {
                let val = if msbf {
                    f64::from_be_bytes([
                        payload[pos],
                        payload[pos + 1],
                        payload[pos + 2],
                        payload[pos + 3],
                        payload[pos + 4],
                        payload[pos + 5],
                        payload[pos + 6],
                        payload[pos + 7],
                    ])
                } else {
                    f64::from_le_bytes([
                        payload[pos],
                        payload[pos + 1],
                        payload[pos + 2],
                        payload[pos + 3],
                        payload[pos + 4],
                        payload[pos + 5],
                        payload[pos + 6],
                        payload[pos + 7],
                    ])
                };
                parts.push(format!("{:.6}", val));
            }
            pos += byte_len;
        } else if is_rawd {
            // Raw data: length(2 bytes) + data
            if pos + 2 > payload.len() {
                break;
            }
            let raw_len = if msbf {
                u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize
            } else {
                u16::from_le_bytes([payload[pos], payload[pos + 1]]) as usize
            };
            pos += 2;
            if pos + raw_len > payload.len() {
                break;
            }
            let hex: Vec<String> = payload[pos..pos + raw_len]
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect();
            parts.push(format!("[{}]", hex.join(" ")));
            pos += raw_len;
        } else {
            // Unknown type - skip remaining
            break;
        }
    }

    parts.join(" ")
}

/// Sanitize raw bytes into displayable text, replacing control chars with '.'
fn sanitize_payload_text(payload_bytes: &[u8]) -> String {
    String::from_utf8_lossy(payload_bytes)
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '.'
            } else {
                c
            }
        })
        .collect()
}

pub fn parse_dlt_message(input: &[u8]) -> Result<(&[u8], DltMessage), ParseError> {
    if input.len() < 4 {
        return Err(ParseError::Incomplete(4 - input.len()));
    }

    // 1. Storage Header (16 bytes: "DLT\x01" + timestamp_sec(4 LE) + timestamp_us(4 LE) + ecu_id(4))
    let storage_res = parse_storage_header(input);
    let (input, (timestamp_us, storage_ecu_id)) = match storage_res {
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

    // 2. Standard Header (4 bytes: HTYP(1) + MCNT(1) + LEN(2 BE))
    let (mut input, (htyp, _mcnt, len)) = match parse_standard_header(input) {
        Ok(res) => res,
        Err(nom::Err::Incomplete(_)) => return Err(ParseError::Incomplete(4)),
        Err(_) => return Err(ParseError::Incomplete(4)),
    };

    // HTYP bit fields:
    // Bit 0: UEH (Use Extended Header)
    // Bit 1: MSBF (MSB First / Big Endian for payload)
    // Bit 2: WEID (With ECU ID)
    // Bit 3: WSID (With Session ID)
    // Bit 4: WTMS (With Timestamp)
    // Bits 5-7: VERS (Version, should be 1)
    let ueh = (htyp & 0x01) != 0;
    let msbf = (htyp & 0x02) != 0;
    let weid = (htyp & 0x04) != 0;
    let wsid = (htyp & 0x08) != 0;
    let wtms = (htyp & 0x10) != 0;

    // Calculate expected bytes after the 4-byte standard header base
    let expected_remaining = (len as usize).saturating_sub(4);
    if input.len() < expected_remaining {
        return Err(ParseError::Incomplete(expected_remaining - input.len()));
    }

    // Track how many bytes of the message body we've consumed for optional fields
    let mut consumed_extra: usize = 0;

    // 2a. Optional ECU ID in standard header
    let mut ecu_id = storage_ecu_id;
    if weid {
        if expected_remaining < consumed_extra + 4 {
            return Err(ParseError::InvalidHeader);
        }
        let (new_input, eid) = match read_4byte_id(input) {
            Ok(res) => res,
            Err(_) => return Err(ParseError::InvalidHeader),
        };
        input = new_input;
        ecu_id = eid; // Override with the one from standard header
        consumed_extra += 4;
    }

    // 2b. Optional Session ID
    if wsid {
        if expected_remaining < consumed_extra + 4 {
            return Err(ParseError::InvalidHeader);
        }
        let (new_input, _sid) = match take::<usize, &[u8], nom::error::Error<&[u8]>>(4usize)(input)
        {
            Ok(res) => res,
            Err(_) => return Err(ParseError::InvalidHeader),
        };
        input = new_input;
        consumed_extra += 4;
    }

    // 2c. Optional Timestamp
    if wtms {
        if expected_remaining < consumed_extra + 4 {
            return Err(ParseError::InvalidHeader);
        }
        let (new_input, _tms) = match take::<usize, &[u8], nom::error::Error<&[u8]>>(4usize)(input)
        {
            Ok(res) => res,
            Err(_) => return Err(ParseError::InvalidHeader),
        };
        input = new_input;
        consumed_extra += 4;
    }

    let mut msg_apid = None;
    let mut msg_ctid = None;
    let mut msg_log_level = None;
    let mut is_verbose = false;
    let mut noar: u8 = 0;

    let payload_start = expected_remaining.saturating_sub(consumed_extra);
    let mut actual_payload_len = payload_start;

    // 3. Extended Header (10 bytes if UEH is set)
    if ueh {
        if actual_payload_len < 10 {
            return Err(ParseError::InvalidHeader);
        }
        let (new_input, (msin, ext_noar, apid, ctid)) = match parse_extended_header(input) {
            Ok(res) => res,
            Err(nom::Err::Incomplete(_)) => return Err(ParseError::Incomplete(10)),
            Err(_) => return Err(ParseError::InvalidHeader),
        };
        input = new_input;
        msg_apid = Some(apid);
        msg_ctid = Some(ctid);
        noar = ext_noar;

        // MSIN bit fields (AUTOSAR DLT PRS):
        // Bit 0: Verbose flag (1 = verbose, 0 = non-verbose)
        // Bits 1-3: MSTP (Message Type: 0=Log, 1=Trace, 2=Network, 3=Control)
        // Bits 4-7: MTIN (Message Type Info, meaning depends on MSTP)
        is_verbose = (msin & 0x01) != 0;
        let mstp = (msin >> 1) & 0x07;

        if mstp == 0 {
            // MSTP = 0: DLT_TYPE_LOG
            // MTIN for Log: 1=Fatal, 2=Error, 3=Warn, 4=Info, 5=Debug, 6=Verbose
            let mtin = (msin >> 4) & 0x0F;
            match mtin {
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

    // 5. Decode payload
    let payload_text = if is_verbose && noar > 0 {
        let decoded = decode_verbose_payload(payload_bytes, noar, msbf);
        if decoded.is_empty() {
            // Fallback to raw display if decoding returned nothing
            sanitize_payload_text(payload_bytes)
        } else {
            decoded
        }
    } else {
        sanitize_payload_text(payload_bytes)
    };

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

    // ==================== Test helpers ====================

    /// Build a spec-compliant DLT message with storage header.
    /// Standard Header LEN is big-endian per AUTOSAR spec.
    /// MSIN uses correct bit layout: bit 0 = verbose, bits 1-3 = MSTP, bits 4-7 = MTIN.
    fn build_spec_compliant_message(payload: &[u8]) -> Vec<u8> {
        build_spec_message_with_options(payload, false, 4, false) // non-verbose, Info, no MSBF
    }

    fn build_verbose_message(payload_args: &[u8]) -> Vec<u8> {
        build_spec_message_with_options(payload_args, true, 4, false) // verbose, Info, LE
    }

    /// Full control message builder.
    /// log_level_mtin: 1=Fatal, 2=Error, 3=Warn, 4=Info, 5=Debug, 6=Verbose
    fn build_spec_message_with_options(
        payload: &[u8],
        verbose: bool,
        log_level_mtin: u8,
        _msbf: bool,
    ) -> Vec<u8> {
        let mut msg = Vec::new();

        // 1. Storage Header (16 bytes)
        msg.extend_from_slice(b"DLT\x01");
        msg.extend_from_slice(&1640995200u32.to_le_bytes()); // timestamp seconds
        msg.extend_from_slice(&123456u32.to_le_bytes()); // timestamp microseconds
        msg.extend_from_slice(b"ECU1"); // ECU ID

        // 2. Standard Header (4 bytes)
        // HTYP: UEH=1 (bit0), MSBF=0 (bit1), WEID=0, WSID=0, WTMS=0, VERS=1 (bits 5-7)
        // => 0b00100001 = 0x21
        let htyp: u8 = 0x21;
        msg.push(htyp);
        msg.push(0x00); // MCNT

        // LEN = Standard Header (4) + Extended Header (10) + Payload
        let total_len: u16 = 4 + 10 + payload.len() as u16;
        msg.extend_from_slice(&total_len.to_be_bytes()); // BIG ENDIAN per spec

        // 3. Extended Header (10 bytes)
        // MSIN: bit 0 = verbose flag, bits 1-3 = MSTP (0=Log), bits 4-7 = MTIN (log level)
        let verbose_bit: u8 = if verbose { 1 } else { 0 };
        let msin: u8 = verbose_bit | (0 << 1) | (log_level_mtin << 4);
        msg.push(msin);
        msg.push(1); // NOAR
        msg.extend_from_slice(b"APP1"); // APID
        msg.extend_from_slice(b"CTX1"); // CTID

        // 4. Payload
        msg.extend_from_slice(payload);

        msg
    }

    /// Build a verbose string argument (TypeInfo + length + string data)
    fn build_verbose_string_arg(s: &str) -> Vec<u8> {
        let mut arg = Vec::new();
        // TypeInfo: STRG bit (bit 9) = 1 => 0x00000200
        let type_info: u32 = 0x0000_0200; // STRG
        arg.extend_from_slice(&type_info.to_le_bytes());
        // Length (2 bytes LE) includes null terminator
        let str_len = (s.len() + 1) as u16;
        arg.extend_from_slice(&str_len.to_le_bytes());
        arg.extend_from_slice(s.as_bytes());
        arg.push(0x00); // null terminator
        arg
    }

    /// Build a verbose uint32 argument
    fn build_verbose_uint32_arg(val: u32) -> Vec<u8> {
        let mut arg = Vec::new();
        // TypeInfo: UINT bit (bit 6) = 1, TYLE = 3 (32-bit) => 0x00000043
        let type_info: u32 = 0x0000_0043; // UINT | TYLE=3
        arg.extend_from_slice(&type_info.to_le_bytes());
        arg.extend_from_slice(&val.to_le_bytes());
        arg
    }

    /// Build a verbose sint32 argument
    fn build_verbose_sint32_arg(val: i32) -> Vec<u8> {
        let mut arg = Vec::new();
        // TypeInfo: SINT bit (bit 5) = 1, TYLE = 3 (32-bit) => 0x00000023
        let type_info: u32 = 0x0000_0023; // SINT | TYLE=3
        arg.extend_from_slice(&type_info.to_le_bytes());
        arg.extend_from_slice(&val.to_le_bytes());
        arg
    }

    // ==================== parse_dlt_message tests ====================

    #[test]
    fn test_parse_valid_non_verbose_message() {
        let data = build_spec_compliant_message(b"Hello DLT");

        let (remaining, msg) = parse_dlt_message(&data).expect("Parsing failed");
        assert_eq!(remaining.len(), 0);
        assert_eq!(msg.ecu_id, "ECU1");
        assert_eq!(msg.apid, Some("APP1".to_string()));
        assert_eq!(msg.ctid, Some("CTX1".to_string()));
        assert_eq!(msg.log_level, Some(LogLevel::Info));
        assert_eq!(msg.payload_text, "Hello DLT");
    }

    #[test]
    fn test_parse_verbose_string_message() {
        let payload = build_verbose_string_arg("Daemon launched. Starting to output traces...");
        let data = build_verbose_message(&payload);

        let (remaining, msg) = parse_dlt_message(&data).expect("Parsing failed");
        assert_eq!(remaining.len(), 0);
        assert_eq!(
            msg.payload_text,
            "Daemon launched. Starting to output traces..."
        );
        assert_eq!(msg.log_level, Some(LogLevel::Info));
    }

    #[test]
    fn test_parse_verbose_uint_message() {
        let payload = build_verbose_uint32_arg(42);
        let data = build_verbose_message(&payload);

        let (_, msg) = parse_dlt_message(&data).expect("Parsing failed");
        assert_eq!(msg.payload_text, "42");
    }

    #[test]
    fn test_parse_verbose_sint_message() {
        let payload = build_verbose_sint32_arg(-123);
        let data = build_verbose_message(&payload);

        let (_, msg) = parse_dlt_message(&data).expect("Parsing failed");
        assert_eq!(msg.payload_text, "-123");
    }

    #[test]
    fn test_parse_verbose_mixed_args() {
        let mut payload = Vec::new();
        payload.extend(build_verbose_string_arg("RPM:"));
        payload.extend(build_verbose_uint32_arg(2400));

        // Build message with NOAR=2
        let mut msg_bytes = Vec::new();
        msg_bytes.extend_from_slice(b"DLT\x01");
        msg_bytes.extend_from_slice(&1640995200u32.to_le_bytes());
        msg_bytes.extend_from_slice(&123456u32.to_le_bytes());
        msg_bytes.extend_from_slice(b"ECU1");
        msg_bytes.push(0x21); // HTYP
        msg_bytes.push(0x00); // MCNT
        let total_len: u16 = 4 + 10 + payload.len() as u16;
        msg_bytes.extend_from_slice(&total_len.to_be_bytes());
        // MSIN: verbose=1, MSTP=0(Log), MTIN=4(Info)
        msg_bytes.push(0x41);
        msg_bytes.push(2); // NOAR = 2
        msg_bytes.extend_from_slice(b"APP1");
        msg_bytes.extend_from_slice(b"CTX1");
        msg_bytes.extend(payload);

        let (_, msg) = parse_dlt_message(&msg_bytes).expect("Parsing failed");
        assert_eq!(msg.payload_text, "RPM: 2400");
    }

    #[test]
    fn test_parse_with_weid_wsid_wtms() {
        // Build a message with WEID=1, WSID=1, WTMS=1 (like real IVI data)
        let mut msg_bytes = Vec::new();

        // Storage Header
        msg_bytes.extend_from_slice(b"DLT\x01");
        msg_bytes.extend_from_slice(&1640995200u32.to_le_bytes());
        msg_bytes.extend_from_slice(&0u32.to_le_bytes());
        msg_bytes.extend_from_slice(b"ECU\0");

        // Standard Header
        // HTYP: UEH=1, MSBF=0, WEID=1, WSID=1, WTMS=1, VERS=1
        // => 0b00111101 = 0x3D
        msg_bytes.push(0x3D);
        msg_bytes.push(0x00); // MCNT
        // LEN = StdHdr(4) + WEID(4) + WSID(4) + WTMS(4) + ExtHdr(10) + Payload
        let payload_data = b"Real IVI log";
        let total_len: u16 = 4 + 4 + 4 + 4 + 10 + payload_data.len() as u16;
        msg_bytes.extend_from_slice(&total_len.to_be_bytes());

        // Optional fields
        msg_bytes.extend_from_slice(b"CIVI"); // ECU ID (WEID)
        msg_bytes.extend_from_slice(&0x0000070Bu32.to_be_bytes()); // Session ID (WSID)
        msg_bytes.extend_from_slice(&0xB4D97D0Bu32.to_be_bytes()); // Timestamp (WTMS)

        // Extended Header
        // MSIN: verbose=0, MSTP=0(Log), MTIN=4(Info) => (4 << 4) | 0 = 0x40
        msg_bytes.push(0x40);
        msg_bytes.push(1); // NOAR
        msg_bytes.extend_from_slice(b"VRBT");
        msg_bytes.extend_from_slice(b"BOOT");

        // Payload (non-verbose)
        msg_bytes.extend_from_slice(payload_data);

        let (remaining, msg) = parse_dlt_message(&msg_bytes).expect("Parsing failed");
        assert_eq!(remaining.len(), 0);
        assert_eq!(msg.ecu_id, "CIVI"); // Should use WEID ECU ID
        assert_eq!(msg.apid, Some("VRBT".to_string()));
        assert_eq!(msg.ctid, Some("BOOT".to_string()));
        assert_eq!(msg.log_level, Some(LogLevel::Info));
        assert_eq!(msg.payload_text, "Real IVI log");
    }

    #[test]
    fn test_parse_invalid_magic_number() {
        let mut data = build_spec_compliant_message(b"test");
        data[0] = b'X';

        let err = parse_dlt_message(&data).unwrap_err();
        assert_eq!(err, ParseError::InvalidMagicNumber);
    }

    #[test]
    fn test_parse_truncated_message() {
        let mut data = build_spec_compliant_message(b"Hello DLT");
        data.truncate(20);

        let err = parse_dlt_message(&data).unwrap_err();
        match err {
            ParseError::Incomplete(_) => {}
            _ => panic!("Expected ParseError::Incomplete, got {:?}", err),
        }
    }

    #[test]
    fn test_parse_all_log_levels() {
        for (mtin, expected) in [
            (1u8, LogLevel::Fatal),
            (2, LogLevel::Error),
            (3, LogLevel::Warn),
            (4, LogLevel::Info),
            (5, LogLevel::Debug),
            (6, LogLevel::Verbose),
            (7, LogLevel::Unknown(7)),
        ] {
            let data = build_spec_message_with_options(b"test", false, mtin, false);
            let (_, msg) = parse_dlt_message(&data).expect("Parsing failed");
            assert_eq!(msg.log_level, Some(expected), "Failed for MTIN={}", mtin);
        }
    }

    // ==================== find_next_sync tests ====================

    #[test]
    fn test_find_next_sync_with_storage_header_magic() {
        let mut data = vec![0x00, 0x00, 0xFF, 0xFF];
        data.extend_from_slice(b"DLT\x01");
        data.extend_from_slice(&[0x00; 12]);

        let pos = find_next_sync(&data).unwrap();
        assert_eq!(pos, 4);
    }

    #[test]
    fn test_find_next_sync_at_start() {
        let mut data = Vec::new();
        data.extend_from_slice(b"DLT\x01");
        data.extend_from_slice(&[0x00; 12]);

        let pos = find_next_sync(&data).unwrap();
        assert_eq!(pos, 0);
    }

    #[test]
    fn test_find_next_sync_no_marker() {
        let data = vec![0x00; 16];
        let pos = find_next_sync(&data);
        assert!(pos.is_none());
    }

    #[test]
    fn test_find_next_sync_with_standard_header_heuristic() {
        let data = vec![0x00, 0x00, 0x21, 0x00, 0x00, 0x17, 0x00];
        let pos = find_next_sync(&data).unwrap();
        assert_eq!(pos, 2);
    }

    // ==================== parse_all_messages tests ====================

    #[test]
    fn test_parse_all_messages_clean_data() {
        let mut data = Vec::new();
        data.extend(build_spec_compliant_message(b"Message 1"));
        data.extend(build_spec_compliant_message(b"Message 2"));
        data.extend(build_spec_compliant_message(b"Message 3"));

        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 3);
        assert_eq!(skipped, 0);
        assert_eq!(msgs[0].payload_text, "Message 1");
        assert_eq!(msgs[1].payload_text, "Message 2");
        assert_eq!(msgs[2].payload_text, "Message 3");
    }

    #[test]
    fn test_parse_all_messages_with_garbage_between() {
        let mut data = Vec::new();
        data.extend(build_spec_compliant_message(b"First"));
        data.extend_from_slice(&[0xFF, 0xFE, 0xFD, 0xFC, 0xFB]);
        data.extend(build_spec_compliant_message(b"Second"));

        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 2);
        assert!(skipped > 0);
        assert_eq!(msgs[0].payload_text, "First");
        assert_eq!(msgs[1].payload_text, "Second");
    }

    #[test]
    fn test_parse_all_messages_with_garbage_prefix() {
        let mut data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        data.extend(build_spec_compliant_message(b"After garbage"));

        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 1);
        assert!(skipped > 0);
        assert_eq!(msgs[0].payload_text, "After garbage");
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
        assert!(skipped > 0);
    }

    // ==================== decode_verbose_payload tests ====================

    #[test]
    fn test_decode_verbose_string() {
        let payload = build_verbose_string_arg("Hello World");
        let result = decode_verbose_payload(&payload, 1, false);
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_decode_verbose_uint32() {
        let payload = build_verbose_uint32_arg(12345);
        let result = decode_verbose_payload(&payload, 1, false);
        assert_eq!(result, "12345");
    }

    #[test]
    fn test_decode_verbose_sint32() {
        let payload = build_verbose_sint32_arg(-42);
        let result = decode_verbose_payload(&payload, 1, false);
        assert_eq!(result, "-42");
    }

    #[test]
    fn test_decode_verbose_multiple_args() {
        let mut payload = Vec::new();
        payload.extend(build_verbose_string_arg("count="));
        payload.extend(build_verbose_uint32_arg(100));
        let result = decode_verbose_payload(&payload, 2, false);
        assert_eq!(result, "count= 100");
    }

    #[test]
    fn test_decode_verbose_empty() {
        let result = decode_verbose_payload(&[], 0, false);
        assert_eq!(result, "");
    }

    // ==================== Scenario: real-world corrupted DLT file ====================

    #[test]
    fn test_scenario_corrupted_dlt_file() {
        let mut data = Vec::new();

        data.extend(build_spec_compliant_message(b"Boot started"));
        data.extend_from_slice(&[0x00, 0x00, 0xFF, 0x44, 0x4C, 0xAB]);
        data.extend(build_spec_compliant_message(b"GPS acquired"));
        data.extend_from_slice(&[0xCA, 0xFE, 0xBA, 0xBE]);
        data.extend(build_spec_compliant_message(b"CAN timeout"));

        let (msgs, skipped) = parse_all_messages(&data);
        assert_eq!(msgs.len(), 3, "All 3 valid messages should be recovered");
        assert!(skipped > 0);
        assert_eq!(msgs[0].payload_text, "Boot started");
        assert_eq!(msgs[1].payload_text, "GPS acquired");
        assert_eq!(msgs[2].payload_text, "CAN timeout");
    }

    /// Scenario: Verbose message like a real IVI system would produce
    #[test]
    fn test_scenario_real_ivi_verbose_message() {
        let arg = build_verbose_string_arg(
            "234:234:cdfw_boot_main.cpp:60:main:Release version.",
        );
        let data = build_verbose_message(&arg);

        let (_, msg) = parse_dlt_message(&data).expect("Should parse");
        assert_eq!(
            msg.payload_text,
            "234:234:cdfw_boot_main.cpp:60:main:Release version."
        );
        assert!(!msg.payload_text.contains('\0'));
        assert!(!msg.payload_text.contains('\u{FFFD}')); // no replacement chars
    }
}
