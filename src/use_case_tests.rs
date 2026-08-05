use crate::{
    app::{App, Filter},
    exporter,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use flate2::{Compression, write::GzEncoder};
use std::{
    fs::{self, File},
    io::Write,
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;
use zip::{ZipWriter, write::SimpleFileOptions};

fn storage_message(timestamp_sec: u32, level: u8, apid: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"DLT\x01");
    bytes.extend_from_slice(&timestamp_sec.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(b"ECU1");
    bytes.extend_from_slice(&[0x21, 0]);
    bytes.extend_from_slice(&(14_u16 + payload.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&[level << 4, 0]);
    bytes.extend_from_slice(apid);
    bytes.extend_from_slice(b"CTX1");
    bytes.extend_from_slice(payload);
    bytes
}

fn raw_message(timestamp_ticks: u32, counter: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x31, counter]);
    bytes.extend_from_slice(&(18_u16 + payload.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&timestamp_ticks.to_be_bytes());
    bytes.extend_from_slice(&[3 << 4, 0]);
    bytes.extend_from_slice(b"LIVE");
    bytes.extend_from_slice(b"TCP1");
    bytes.extend_from_slice(payload);
    bytes
}

fn wait_for_load(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.is_loading && Instant::now() < deadline {
        app.on_tick();
        thread::sleep(Duration::from_millis(1));
    }
    app.on_tick();
    assert!(!app.is_loading, "load did not finish within five seconds");
}

fn reset_external_config(app: &mut App) {
    app.filter = Filter::default();
    app.info_message = None;
    app.error_message = None;
    app.apply_filter();
}

fn enter_filter(app: &mut App, opener: char, value: &str) {
    app.handle_key(KeyEvent::new(KeyCode::Char(opener), KeyModifiers::NONE), 20);
    for character in value.chars() {
        app.handle_key(
            KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
            20,
        );
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 20);
}

#[test]
fn offline_triage_loads_filters_and_exports_mixed_captures() {
    let temp = tempdir().unwrap();
    let raw_path = temp.path().join("triage.dlt");
    let gzip_path = temp.path().join("triage.dlt.gz");
    let zip_path = temp.path().join("triage.dlt.zip");

    fs::write(
        &raw_path,
        storage_message(30, 4, b"NOIS", b"background ignored"),
    )
    .unwrap();

    let mut gzip = GzEncoder::new(File::create(&gzip_path).unwrap(), Compression::default());
    gzip.write_all(&storage_message(10, 2, b"DIAG", b"network timeout 42"))
        .unwrap();
    gzip.finish().unwrap();

    let mut zip = ZipWriter::new(File::create(&zip_path).unwrap());
    zip.start_file("trace.dlt", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&storage_message(20, 3, b"DIAG", b"network recovered"))
        .unwrap();
    zip.finish().unwrap();

    let mut app = App::new();
    app.load_files(vec![raw_path, gzip_path, zip_path]).unwrap();
    reset_external_config(&mut app);
    wait_for_load(&mut app);

    assert_eq!(app.logs.len(), 3);
    assert_eq!(app.filtered_log_indices.len(), 3);
    assert_eq!(app.skipped_bytes, 0);
    assert!(app.error_message.is_none());

    let ordered: Vec<_> = app
        .filtered_log_indices
        .iter()
        .map(|&index| {
            let entry = &app.logs[index];
            (entry.message.payload_text(), entry.source_name())
        })
        .collect();
    assert_eq!(
        ordered,
        vec![
            ("network timeout 42", "triage.dlt.gz"),
            ("network recovered", "triage.dlt.zip"),
            ("background ignored", "triage.dlt"),
        ]
    );

    enter_filter(&mut app, 'l', "warn");
    assert_eq!(app.filtered_log_indices.len(), 2);
    enter_filter(&mut app, 'a', "diag");
    assert_eq!(app.filtered_log_indices.len(), 2);
    enter_filter(&mut app, '/', "timeout [0-9]+");
    assert_eq!(app.filtered_log_indices.len(), 1);

    let filtered: Vec<_> = app
        .filtered_log_indices
        .iter()
        .map(|&index| &app.logs[index].message)
        .collect();
    let export_path = temp.path().join("filtered.txt");
    exporter::export_to_txt(&filtered, export_path.to_str().unwrap()).unwrap();
    let exported = fs::read_to_string(export_path).unwrap();
    assert!(exported.contains("network timeout 42"));
    assert!(!exported.contains("network recovered"));
    assert!(!exported.contains("background ignored"));
}

#[test]
fn live_tcp_receives_fragmented_messages_and_disconnects_cleanly() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let first = raw_message(20, 0, b"live second");
    let second = raw_message(10, 1, b"live first");
    let first_len = first.len();
    let mut stream_bytes = first;
    stream_bytes.extend_from_slice(&second);

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        for chunk in [
            &stream_bytes[..7],
            &stream_bytes[7..first_len + 3],
            &stream_bytes[first_len + 3..],
        ] {
            stream.write_all(chunk).unwrap();
            thread::sleep(Duration::from_millis(10));
        }
    });

    let mut app = App::new();
    app.connect_tcp(&address.to_string());
    reset_external_config(&mut app);
    wait_for_load(&mut app);
    server.join().unwrap();

    assert_eq!(app.logs.len(), 2);
    assert_eq!(
        app.filtered_log_indices
            .iter()
            .map(|&index| app.logs[index].message.payload_text())
            .collect::<Vec<_>>(),
        vec!["live first", "live second"]
    );
    assert_eq!(app.logs_selected_index, 1);
    assert!(app.auto_scroll);
    assert!(app.connection_info.is_none());
    assert!(app.error_message.is_none());
    assert_eq!(app.skipped_bytes, 0);
}
