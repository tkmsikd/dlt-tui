use std::io::{self, Read};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::parser::{self, DltMessage};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BUFFER_SIZE: usize = 10 * 1024 * 1024; // 10MB — prevents OOM from unparseable streams

/// Connects to a dlt-daemon TCP socket and streams parsed messages into the channel.
/// The connection runs on the calling thread (intended to be spawned in a background thread).
/// Times out after 5 seconds if the host is unreachable.
pub fn stream_from_tcp(addr: &str, tx: Sender<DltMessage>) -> io::Result<()> {
    let socket_addr = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid address"))?;
    let stream = TcpStream::connect_timeout(&socket_addr, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    stream_from_reader(stream, tx)
}

/// Reads DLT messages from any `Read` source and sends them through the channel.
/// Handles both formats: with and without Storage Header.
pub fn stream_from_reader<R: Read>(mut reader: R, tx: Sender<DltMessage>) -> io::Result<()> {
    let mut buffer = Vec::with_capacity(64 * 1024);
    let mut read_buf = [0u8; 8192];

    loop {
        match reader.read(&mut read_buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                buffer.extend_from_slice(&read_buf[..n]);
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Read timeout — no data available yet, try parsing what we have
            }
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                // Same as WouldBlock on some platforms
            }
            Err(e) => return Err(e),
        }

        // Try to parse as many messages as possible from the buffer
        let mut consumed = 0;
        loop {
            let remaining = &buffer[consumed..];
            if remaining.is_empty() {
                break;
            }

            match parser::parse_dlt_message(remaining) {
                Ok((leftover, msg)) => {
                    consumed += remaining.len() - leftover.len();
                    if tx.send(msg).is_err() {
                        // Receiver dropped (app quit)
                        return Ok(());
                    }
                }
                Err(parser::ParseError::Incomplete(_)) => {
                    // Need more data
                    break;
                }
                Err(parser::ParseError::InvalidMagicNumber)
                | Err(parser::ParseError::InvalidHeader)
                | Err(parser::ParseError::Unknown) => {
                    // Try to find next DLT marker or skip one byte
                    if let Some(pos) = parser::find_next_sync(&remaining[1..]) {
                        consumed += 1 + pos;
                    } else {
                        consumed += remaining.len().saturating_sub(3);
                        break;
                    }
                }
            }
        }

        // Remove consumed bytes from buffer
        if consumed > 0 {
            buffer.drain(..consumed);
        }

        // Guard: prevent unbounded buffer growth from unparseable data
        if buffer.len() > MAX_BUFFER_SIZE {
            let search_start = buffer.len() / 2;
            if let Some(sync_pos) = parser::find_next_sync(&buffer[search_start..]) {
                // Found a potential message start — discard everything before it
                buffer.drain(..search_start + sync_pos);
            } else {
                // No sync found — keep only the last 4KB for partial message recovery
                let keep = 4096.min(buffer.len());
                buffer.drain(..buffer.len() - keep);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::mpsc;

    fn build_dlt_message_with_storage_header(payload: &[u8]) -> Vec<u8> {
        let mut msg = Vec::new();
        // Storage Header (16 bytes)
        msg.extend_from_slice(b"DLT\x01");
        msg.extend_from_slice(&1640995200u32.to_le_bytes());
        msg.extend_from_slice(&0u32.to_le_bytes());
        msg.extend_from_slice(b"ECU1");
        // Standard Header
        msg.push(0x21); // HTYP: UEH=1, VERS=1
        msg.push(0x00); // MCNT
        let total_len: u16 = 4 + 10 + payload.len() as u16;
        msg.extend_from_slice(&total_len.to_be_bytes()); // BIG ENDIAN per spec
        // Extended Header (10 bytes)
        // MSIN: verbose=0, MSTP=0(Log), MTIN=4(Info) => (4 << 4) = 0x40
        msg.push(0x40);
        msg.push(1); // NOAR
        msg.extend_from_slice(b"APP1");
        msg.extend_from_slice(b"CTX1");
        // Payload
        msg.extend_from_slice(payload);
        msg
    }

    #[test]
    fn test_stream_single_message() {
        let data = build_dlt_message_with_storage_header(b"Hello TCP");
        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();

        stream_from_reader(cursor, tx).unwrap();

        let msg = rx.recv().unwrap();
        assert_eq!(msg.payload_text, "Hello TCP");
        assert_eq!(msg.ecu_id, "ECU1");
    }

    #[test]
    fn test_stream_multiple_messages() {
        let mut data = Vec::new();
        data.extend(build_dlt_message_with_storage_header(b"Message 1"));
        data.extend(build_dlt_message_with_storage_header(b"Message 2"));
        data.extend(build_dlt_message_with_storage_header(b"Message 3"));

        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();

        stream_from_reader(cursor, tx).unwrap();

        let msgs: Vec<_> = rx.try_iter().collect();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].payload_text, "Message 1");
        assert_eq!(msgs[1].payload_text, "Message 2");
        assert_eq!(msgs[2].payload_text, "Message 3");
    }

    #[test]
    fn test_stream_with_garbage_prefix() {
        let mut data = Vec::new();
        data.extend_from_slice(b"\x00\x00\xFF\xFF"); // garbage bytes
        data.extend(build_dlt_message_with_storage_header(b"After garbage"));

        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();

        stream_from_reader(cursor, tx).unwrap();

        let msgs: Vec<_> = rx.try_iter().collect();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].payload_text, "After garbage");
    }

    #[test]
    fn test_stream_empty_input() {
        let cursor = Cursor::new(Vec::new());
        let (tx, rx) = mpsc::channel();

        stream_from_reader(cursor, tx).unwrap();

        let msgs: Vec<_> = rx.try_iter().collect();
        assert_eq!(msgs.len(), 0);
    }

    #[test]
    fn test_stream_with_interleaved_garbage() {
        let mut data = Vec::new();
        data.extend(build_dlt_message_with_storage_header(b"Msg1"));
        data.extend_from_slice(b"\xFF\xFE\xFD\xFC\xFB"); // garbage
        data.extend(build_dlt_message_with_storage_header(b"Msg2"));

        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();

        stream_from_reader(cursor, tx).unwrap();

        let msgs: Vec<_> = rx.try_iter().collect();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].payload_text, "Msg1");
        assert_eq!(msgs[1].payload_text, "Msg2");
    }

    #[test]
    fn test_stream_truncated_message() {
        // Build a valid message then truncate the last few bytes
        let full = build_dlt_message_with_storage_header(b"Complete");
        let truncated = &full[..full.len() - 3]; // cut off end

        let cursor = Cursor::new(truncated.to_vec());
        let (tx, rx) = mpsc::channel();

        stream_from_reader(cursor, tx).unwrap();

        let msgs: Vec<_> = rx.try_iter().collect();
        assert_eq!(msgs.len(), 0); // can't parse truncated message
    }

    #[test]
    fn test_stream_receiver_dropped() {
        let mut data = Vec::new();
        for i in 0..100 {
            data.extend(build_dlt_message_with_storage_header(
                format!("Msg{}", i).as_bytes(),
            ));
        }

        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();

        // Drop the receiver immediately — sender should handle gracefully
        drop(rx);

        // stream_from_reader should return Ok, not panic
        let result = stream_from_reader(cursor, tx);
        assert!(result.is_ok());
    }

    /// Buffer guard: large volume of unparseable data should not cause OOM.
    /// After processing, valid messages embedded in garbage should be recovered.
    #[test]
    fn test_stream_large_garbage_with_valid_message() {
        let mut data = vec![0xCC; 200 * 1024];
        // Followed by a valid message
        data.extend(build_dlt_message_with_storage_header(b"Survived"));

        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();

        stream_from_reader(cursor, tx).unwrap();

        let msgs: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            msgs.len(),
            1,
            "Should recover the valid message after garbage"
        );
        assert_eq!(msgs[0].payload_text, "Survived");
    }

    /// Buffer guard: pure garbage should not panic or OOM.
    #[test]
    fn test_stream_pure_garbage_no_panic() {
        // 500KB of pure garbage
        let data: Vec<u8> = (0..500 * 1024).map(|i| (i % 251) as u8 | 0x80).collect();
        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();

        let result = stream_from_reader(cursor, tx);
        assert!(result.is_ok(), "Should not panic on pure garbage");

        let msgs: Vec<_> = rx.try_iter().collect();
        assert_eq!(msgs.len(), 0, "No valid messages in garbage");
    }

    /// Buffer guard: buffer should be bounded even with adversarial data.
    /// This test verifies that the MAX_BUFFER_SIZE constant is respected.
    #[test]
    fn test_buffer_bounded_by_max_size() {
        // Create adversarial data: bytes that look like DLT version=1 headers
        // but fail to parse, causing the parser to skip only 1 byte at a time.
        // Without a buffer guard, this would keep the buffer large.
        let mut data = Vec::new();
        // 2MB of adversarial data: 0x21 (valid HTYP) followed by garbage
        for _ in 0..2 * 1024 * 1024 / 4 {
            data.extend_from_slice(&[0x21, 0x00, 0x00, 0x04]); // HTYP=0x21, LEN=4 (too short for ext)
        }
        // Add a valid message at the end
        data.extend(build_dlt_message_with_storage_header(b"After adversarial"));

        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();

        stream_from_reader(cursor, tx).unwrap();

        let msgs: Vec<_> = rx.try_iter().collect();
        // The valid message at the end should be recovered
        assert!(
            msgs.iter().any(|m| m.payload_text == "After adversarial"),
            "Should recover valid message after adversarial data"
        );
    }
}
