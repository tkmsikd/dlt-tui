use std::io::{self, Read};
use std::net::TcpStream;
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::parser::{self, DltMessage};

/// Connects to a dlt-daemon TCP socket and streams parsed messages into the channel.
/// The connection runs on the calling thread (intended to be spawned in a background thread).
pub fn stream_from_tcp(addr: &str, tx: Sender<DltMessage>) -> io::Result<()> {
    let stream = TcpStream::connect(addr)?;
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
}
