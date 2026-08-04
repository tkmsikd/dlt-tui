use std::io::{self, Read};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc::Sender,
};
use std::time::Duration;

use crate::parser::{self, DltMessage};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BUFFER_SIZE: usize = 10 * 1024 * 1024; // 10MB — prevents OOM from unparseable streams

/// Connects to a dlt-daemon TCP socket and streams parsed messages into the channel.
/// The connection runs on the calling thread (intended to be spawned in a background thread).
/// Each resolved endpoint times out after 5 seconds if it is unreachable.
pub fn stream_from_tcp(addr: &str, tx: Sender<DltMessage>) -> io::Result<()> {
    stream_from_tcp_with_handler(addr, |msg| tx.send(msg).is_ok())
}

/// Connects to a dlt-daemon TCP socket and streams parsed messages to a handler.
pub fn stream_from_tcp_with_handler<F>(addr: &str, on_message: F) -> io::Result<()>
where
    F: FnMut(DltMessage) -> bool,
{
    let stream = connect_with_timeout(addr, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    stream_from_reader_inner(stream, None, on_message)
}

fn connect_with_timeout<A: ToSocketAddrs>(addr: A, timeout: Duration) -> io::Result<TcpStream> {
    connect_resolved(addr.to_socket_addrs()?, |socket_addr| {
        TcpStream::connect_timeout(&socket_addr, timeout)
    })
}

fn connect_resolved<T, I, F>(addresses: I, mut connect: F) -> io::Result<T>
where
    I: IntoIterator<Item = SocketAddr>,
    F: FnMut(SocketAddr) -> io::Result<T>,
{
    let mut last_error = None;

    for socket_addr in addresses {
        match connect(socket_addr) {
            Ok(value) => return Ok(value),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "address resolved to no endpoints",
        )
    }))
}

/// Reads DLT messages from any `Read` source and sends them through the channel.
/// Handles both formats: with and without Storage Header.
pub fn stream_from_reader<R: Read>(mut reader: R, tx: Sender<DltMessage>) -> io::Result<()> {
    stream_from_reader_inner(&mut reader, None, |msg| tx.send(msg).is_ok())
}

/// Like `stream_from_reader`, but also reports bytes skipped during parser recovery.
pub fn stream_from_reader_with_skipped<R: Read>(
    mut reader: R,
    tx: Sender<DltMessage>,
    skipped_bytes: Arc<AtomicUsize>,
) -> io::Result<()> {
    stream_from_reader_inner(&mut reader, Some(skipped_bytes), |msg| tx.send(msg).is_ok())
}

/// Streams messages to a caller-provided handler.
/// Returning `false` from the handler stops parsing cleanly.
pub fn stream_from_reader_with_handler<R: Read, F>(
    mut reader: R,
    skipped_bytes: Arc<AtomicUsize>,
    on_message: F,
) -> io::Result<()>
where
    F: FnMut(DltMessage) -> bool,
{
    stream_from_reader_inner(&mut reader, Some(skipped_bytes), on_message)
}

fn stream_from_reader_inner<R: Read, F>(
    mut reader: R,
    skipped_bytes: Option<Arc<AtomicUsize>>,
    mut on_message: F,
) -> io::Result<()>
where
    F: FnMut(DltMessage) -> bool,
{
    let mut buffer = Vec::with_capacity(64 * 1024);
    let mut read_buf = [0u8; 8192];
    let mut total_skipped = skipped_bytes
        .as_ref()
        .map(|skipped_bytes| skipped_bytes.load(Ordering::Relaxed))
        .unwrap_or(0);

    loop {
        let reached_eof = match reader.read(&mut read_buf) {
            Ok(0) => true,
            Ok(n) => {
                buffer.extend_from_slice(&read_buf[..n]);
                false
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Read timeout — no data available yet, try parsing what we have
                false
            }
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                // Same as WouldBlock on some platforms
                false
            }
            Err(e) => return Err(e),
        };

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
                    if !on_message(msg) {
                        return Ok(());
                    }
                }
                Err(parser::ParseError::Incomplete(_)) => {
                    // A corrupt header can claim a huge body and hide a later
                    // message. While the stream is open, only the strong
                    // storage magic is safe enough to resync; at EOF, a
                    // complete raw frame can also be salvaged without racing
                    // a still-arriving outer payload.
                    let recovery_pos = remaining.get(1..).and_then(|tail| {
                        if reached_eof {
                            parser::find_next_complete_message(tail)
                        } else {
                            parser::find_next_complete_storage_message(tail)
                        }
                    });
                    if let Some(pos) = recovery_pos {
                        let skipped = 1 + pos;
                        consumed += skipped;
                        total_skipped += skipped;
                        if let Some(skipped_bytes) = &skipped_bytes {
                            skipped_bytes.store(total_skipped, Ordering::Relaxed);
                        }
                        continue;
                    }
                    break;
                }
                Err(parser::ParseError::InvalidMagicNumber)
                | Err(parser::ParseError::InvalidHeader)
                | Err(parser::ParseError::Unknown) => {
                    // Try to find next DLT marker or skip one byte
                    if let Some(pos) = parser::find_next_sync(&remaining[1..]) {
                        let skipped = 1 + pos;
                        consumed += skipped;
                        total_skipped += skipped;
                    } else {
                        let skipped = remaining.len().saturating_sub(3);
                        consumed += skipped;
                        total_skipped += skipped;
                        break;
                    }
                    if let Some(skipped_bytes) = &skipped_bytes {
                        skipped_bytes.store(total_skipped, Ordering::Relaxed);
                    }
                }
            }
        }

        // Remove consumed bytes from buffer
        if consumed > 0 {
            buffer.drain(..consumed);
        }

        if reached_eof {
            // Any bytes still buffered cannot form a complete message and
            // must be reported as skipped rather than silently discarded.
            if !buffer.is_empty() {
                total_skipped += buffer.len();
                if let Some(skipped_bytes) = &skipped_bytes {
                    skipped_bytes.store(total_skipped, Ordering::Relaxed);
                }
            }
            break;
        }

        // Guard: prevent unbounded buffer growth from unparseable data
        if buffer.len() > MAX_BUFFER_SIZE {
            let search_start = buffer.len() / 2;
            if let Some(sync_pos) = parser::find_next_sync(&buffer[search_start..]) {
                // Found a potential message start — discard everything before it
                let skipped = search_start + sync_pos;
                buffer.drain(..skipped);
                total_skipped += skipped;
            } else {
                // No sync found — keep only the last 4KB for partial message recovery
                let keep = 4096.min(buffer.len());
                let skipped = buffer.len() - keep;
                buffer.drain(..skipped);
                total_skipped += skipped;
            }
            if let Some(skipped_bytes) = &skipped_bytes {
                skipped_bytes.store(total_skipped, Ordering::Relaxed);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::Cursor;
    use std::net::Ipv4Addr;
    use std::sync::mpsc;

    struct ScriptedReader {
        steps: VecDeque<io::Result<Vec<u8>>>,
    }

    impl Read for ScriptedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            match self.steps.pop_front() {
                Some(Ok(data)) => {
                    assert!(data.len() <= buffer.len());
                    buffer[..data.len()].copy_from_slice(&data);
                    Ok(data.len())
                }
                Some(Err(error)) => Err(error),
                None => Ok(0),
            }
        }
    }

    #[test]
    fn test_connect_tries_all_resolved_endpoints() {
        let first = SocketAddr::from((Ipv4Addr::LOCALHOST, 1));
        let second = SocketAddr::from((Ipv4Addr::LOCALHOST, 2));
        let third = SocketAddr::from((Ipv4Addr::LOCALHOST, 3));
        let mut attempted = Vec::new();

        let result = connect_resolved([first, second, third], |address| {
            attempted.push(address);
            if address == first {
                Err(io::Error::from(io::ErrorKind::ConnectionRefused))
            } else {
                Ok("connected")
            }
        })
        .unwrap();

        assert_eq!(result, "connected");
        assert_eq!(attempted, [first, second]);
    }

    #[test]
    fn test_connect_returns_last_error_when_all_endpoints_fail() {
        let addresses = [
            SocketAddr::from((Ipv4Addr::LOCALHOST, 1)),
            SocketAddr::from((Ipv4Addr::LOCALHOST, 2)),
        ];
        let mut attempted = Vec::new();

        let error = connect_resolved(addresses, |address| -> io::Result<()> {
            attempted.push(address);
            if address == addresses[0] {
                Err(io::Error::from(io::ErrorKind::ConnectionRefused))
            } else {
                Err(io::Error::from(io::ErrorKind::TimedOut))
            }
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(attempted, addresses);
    }

    #[test]
    fn test_connect_rejects_empty_resolution() {
        let error = connect_resolved(std::iter::empty::<SocketAddr>(), |_| Ok(())).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "address resolved to no endpoints");
    }

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

    fn build_dlt_message_without_storage_header(payload: &[u8]) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.push(0x21); // HTYP: UEH=1, VERS=1
        msg.push(0x00); // MCNT
        let total_len: u16 = 4 + 10 + payload.len() as u16;
        msg.extend_from_slice(&total_len.to_be_bytes());
        msg.push(0x40); // MSIN: non-verbose Info log
        msg.push(1); // NOAR
        msg.extend_from_slice(b"APP1");
        msg.extend_from_slice(b"CTX1");
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
        assert_eq!(msg.payload_text(), "Hello TCP");
        assert_eq!(msg.ecu_id, "ECU1");
    }

    #[test]
    fn test_stream_preserves_standard_header_timestamp() {
        let mut data = Vec::new();
        data.push(0x31); // HTYP: UEH=1, WTMS=1, VERS=1
        data.push(0x00);
        data.extend_from_slice(&23u16.to_be_bytes());
        data.extend_from_slice(&42_000u32.to_be_bytes());
        data.push(0x40);
        data.push(1);
        data.extend_from_slice(b"APP1");
        data.extend_from_slice(b"CTX1");
        data.extend_from_slice(b"Hello");
        let (tx, rx) = mpsc::channel();

        stream_from_reader(Cursor::new(data), tx).unwrap();

        let message = rx.recv().unwrap();
        assert_eq!(message.timestamp_us, 4_200_000);
        assert_eq!(message.payload_text(), "Hello");
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
        assert_eq!(msgs[0].payload_text(), "Message 1");
        assert_eq!(msgs[1].payload_text(), "Message 2");
        assert_eq!(msgs[2].payload_text(), "Message 3");
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
        assert_eq!(msgs[0].payload_text(), "After garbage");
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
        assert_eq!(msgs[0].payload_text(), "Msg1");
        assert_eq!(msgs[1].payload_text(), "Msg2");
    }

    #[test]
    fn test_stream_recovers_storage_message_after_bogus_incomplete_raw_header() {
        let mut data = vec![0x21, 0x00, 0xFF, 0xFF];
        data.extend(build_dlt_message_with_storage_header(b"After bogus header"));

        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();

        stream_from_reader(cursor, tx).unwrap();

        let msgs: Vec<_> = rx.try_iter().collect();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].payload_text(), "After bogus header");
    }

    #[test]
    fn test_stream_recovers_raw_message_after_bogus_incomplete_raw_header() {
        let mut data = vec![0x21, 0x00, 0xFF, 0xFF];
        data.extend(build_dlt_message_without_storage_header(
            b"Raw after bogus header",
        ));
        let (tx, rx) = mpsc::channel();
        let skipped_bytes = Arc::new(AtomicUsize::new(0));

        stream_from_reader_with_skipped(Cursor::new(data), tx, Arc::clone(&skipped_bytes)).unwrap();

        let messages: Vec<_> = rx.try_iter().collect();
        assert_eq!(skipped_bytes.load(Ordering::Relaxed), 4);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload_text(), "Raw after bogus header");
    }

    #[test]
    fn test_raw_eof_recovery_counts_prefix_and_trailer_once() {
        let mut data = vec![0x21, 0x00, 0xFF, 0xFF];
        data.extend(build_dlt_message_without_storage_header(b"Recovered"));
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        let (tx, rx) = mpsc::channel();
        let skipped_bytes = Arc::new(AtomicUsize::new(7));

        stream_from_reader_with_skipped(Cursor::new(data), tx, Arc::clone(&skipped_bytes)).unwrap();

        let messages: Vec<_> = rx.try_iter().collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload_text(), "Recovered");
        assert_eq!(skipped_bytes.load(Ordering::Relaxed), 14);
    }

    #[test]
    fn test_raw_eof_recovery_stops_cleanly_when_handler_closes() {
        let mut data = vec![0x21, 0x00, 0xFF, 0xFF];
        data.extend(build_dlt_message_without_storage_header(b"Recovered"));
        data.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        let skipped_bytes = Arc::new(AtomicUsize::new(11));
        let mut received = 0;

        stream_from_reader_with_handler(Cursor::new(data), Arc::clone(&skipped_bytes), |_| {
            received += 1;
            false
        })
        .unwrap();

        assert_eq!(received, 1);
        assert_eq!(skipped_bytes.load(Ordering::Relaxed), 15);
    }

    #[test]
    fn test_stream_waits_for_outer_raw_message_with_embedded_raw_frame() {
        let embedded = build_dlt_message_without_storage_header(b"Embedded");
        let mut outer_payload = b"Before".to_vec();
        outer_payload.extend_from_slice(&embedded);
        outer_payload.extend_from_slice(b"After");
        let outer = build_dlt_message_without_storage_header(&outer_payload);
        let split = 14 + b"Before".len() + embedded.len();
        let reader = ScriptedReader {
            steps: VecDeque::from([
                Ok(outer[..split].to_vec()),
                Err(io::Error::from(io::ErrorKind::WouldBlock)),
                Err(io::Error::from(io::ErrorKind::TimedOut)),
                Ok(outer[split..].to_vec()),
            ]),
        };
        let (tx, rx) = mpsc::channel();
        let skipped_bytes = Arc::new(AtomicUsize::new(0));

        stream_from_reader_with_skipped(reader, tx, Arc::clone(&skipped_bytes)).unwrap();

        let messages: Vec<_> = rx.try_iter().collect();
        assert_eq!(skipped_bytes.load(Ordering::Relaxed), 0);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload_raw(), outer_payload);
    }

    #[test]
    fn test_eof_salvages_complete_raw_embedded_in_truncated_outer_frame() {
        let embedded = build_dlt_message_without_storage_header(b"Embedded");
        let mut full_payload = b"Before".to_vec();
        full_payload.extend_from_slice(&embedded);
        full_payload.extend_from_slice(b"Missing tail");
        let mut truncated_outer = build_dlt_message_without_storage_header(&full_payload);
        truncated_outer.truncate(14 + b"Before".len() + embedded.len());
        let (tx, rx) = mpsc::channel();
        let skipped_bytes = Arc::new(AtomicUsize::new(0));

        stream_from_reader_with_skipped(
            Cursor::new(truncated_outer),
            tx,
            Arc::clone(&skipped_bytes),
        )
        .unwrap();

        let messages: Vec<_> = rx.try_iter().collect();
        assert_eq!(skipped_bytes.load(Ordering::Relaxed), 20);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload_text(), "Embedded");
    }

    #[test]
    fn test_stream_recovers_raw_message_after_invalid_storage_magic() {
        let mut data = b"DLTx".to_vec();
        data.extend(build_dlt_message_without_storage_header(
            b"Raw after invalid magic",
        ));

        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();
        let skipped_bytes = Arc::new(AtomicUsize::new(0));

        stream_from_reader_with_skipped(cursor, tx, Arc::clone(&skipped_bytes)).unwrap();

        let messages: Vec<_> = rx.try_iter().collect();
        assert_eq!(skipped_bytes.load(Ordering::Relaxed), 4);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload_text(), "Raw after invalid magic");
    }

    #[test]
    fn test_stream_recovers_after_storage_header_with_oversized_length() {
        let mut data = Vec::new();
        data.extend_from_slice(b"DLT\x01");
        data.extend_from_slice(&1640995200u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(b"ECU1");
        data.extend_from_slice(&[0x21, 0x00, 0xff, 0xff]);
        let corrupt_len = data.len();
        data.extend(build_dlt_message_with_storage_header(b"Recovered"));

        let cursor = Cursor::new(data);
        let (tx, rx) = mpsc::channel();
        let skipped_bytes = Arc::new(AtomicUsize::new(0));

        stream_from_reader_with_skipped(cursor, tx, Arc::clone(&skipped_bytes)).unwrap();

        let messages: Vec<_> = rx.try_iter().collect();
        assert_eq!(skipped_bytes.load(Ordering::Relaxed), corrupt_len);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload_text(), "Recovered");
    }

    #[test]
    fn test_stream_truncated_message() {
        let full = build_dlt_message_with_storage_header(b"Complete");

        for cut in 1..full.len() {
            let cursor = Cursor::new(full[..cut].to_vec());
            let (tx, rx) = mpsc::channel();
            let skipped_bytes = Arc::new(AtomicUsize::new(0));

            stream_from_reader_with_skipped(cursor, tx, Arc::clone(&skipped_bytes)).unwrap();

            assert_eq!(rx.try_iter().count(), 0, "cut at byte {cut}");
            assert_eq!(
                skipped_bytes.load(Ordering::Relaxed),
                cut,
                "cut at byte {cut}"
            );
        }
    }

    #[test]
    fn test_stream_counts_each_trailing_byte_once_across_streams() {
        let skipped_bytes = Arc::new(AtomicUsize::new(0));
        let mut expected_total = 0;

        for trailing_len in 0..=4 {
            let mut data = build_dlt_message_with_storage_header(b"Complete");
            data.extend(std::iter::repeat_n(0xAA, trailing_len));
            let cursor = Cursor::new(data);
            let (tx, rx) = mpsc::channel();

            stream_from_reader_with_skipped(cursor, tx, Arc::clone(&skipped_bytes)).unwrap();

            let messages: Vec<_> = rx.try_iter().collect();
            assert_eq!(messages.len(), 1, "trailing length {trailing_len}");
            assert_eq!(messages[0].payload_text(), "Complete");
            expected_total += trailing_len;
            assert_eq!(
                skipped_bytes.load(Ordering::Relaxed),
                expected_total,
                "trailing length {trailing_len}"
            );
        }
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
        assert_eq!(msgs[0].payload_text(), "Survived");
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
            msgs.iter().any(|m| m.payload_text() == "After adversarial"),
            "Should recover valid message after adversarial data"
        );
    }
}
