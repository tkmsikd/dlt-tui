/// Generates a sample DLT file for manual testing.
/// Run with: cargo test --test generate_sample_dlt -- --nocapture
use std::io::Write;

fn build_dlt_message(ecu: &str, apid: &str, ctid: &str, log_level: u8, payload: &str) -> Vec<u8> {
    let mut msg = Vec::new();

    // 1. Storage Header (16 bytes)
    msg.extend_from_slice(b"DLT\x01"); // Magic number
    msg.extend_from_slice(&1640995200u32.to_le_bytes()); // timestamp seconds
    msg.extend_from_slice(&123456u32.to_le_bytes()); // timestamp microseconds

    // ECU ID (4 bytes, padded with null)
    let mut ecu_bytes = [0u8; 4];
    for (i, b) in ecu.bytes().enumerate().take(4) {
        ecu_bytes[i] = b;
    }
    msg.extend_from_slice(&ecu_bytes);

    // 2. Standard Header
    // HTYP: UEH(bit0)=1, VERS(bit5-7)=1 => 0x21
    msg.push(0x21);
    msg.push(0x00); // MCNT

    let payload_bytes = payload.as_bytes();
    let total_len: u16 = 4 + 10 + payload_bytes.len() as u16; // StdHdr + ExtHdr + Payload
    msg.extend_from_slice(&total_len.to_le_bytes());

    // 3. Extended Header (10 bytes)
    // MSIN: Message Type = 0 (Log), log_level shifted left by 3
    let msin = log_level << 3;
    msg.push(msin);
    msg.push(1); // NOAR

    let mut apid_bytes = [0u8; 4];
    for (i, b) in apid.bytes().enumerate().take(4) {
        apid_bytes[i] = b;
    }
    msg.extend_from_slice(&apid_bytes);

    let mut ctid_bytes = [0u8; 4];
    for (i, b) in ctid.bytes().enumerate().take(4) {
        ctid_bytes[i] = b;
    }
    msg.extend_from_slice(&ctid_bytes);

    // 4. Payload
    msg.extend_from_slice(payload_bytes);

    msg
}

#[test]
fn generate_sample_dlt_file() {
    let out_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sample.dlt");

    let messages = vec![
        (
            "ECU1",
            "SYS",
            "BOOT",
            4,
            "System boot completed successfully.",
        ),
        (
            "ECU1",
            "NAV",
            "GPS",
            4,
            "GPS signal acquired: lat=35.6812, lon=139.7671",
        ),
        (
            "ECU1",
            "NAV",
            "MAP",
            5,
            "Loading map tile for region Tokyo-Central",
        ),
        (
            "ECU1",
            "DIAG",
            "CAN",
            3,
            "CAN bus timeout on interface vcan0",
        ),
        (
            "ECU1",
            "SYS",
            "MEM",
            2,
            "Memory usage critical: 95% of 4GB used",
        ),
        (
            "ECU1",
            "NAV",
            "GPS",
            4,
            "Position update: speed=60km/h, heading=NNE",
        ),
        (
            "ECU1",
            "DIAG",
            "OBD",
            4,
            "OBD-II PID query: engine RPM = 2400",
        ),
        (
            "ECU1",
            "DIAG",
            "CAN",
            3,
            "CAN frame drop detected: message_id=0x1A3",
        ),
        (
            "ECU1",
            "SYS",
            "NET",
            4,
            "WiFi connected to SSID: VehicleNet_5G",
        ),
        (
            "ECU1",
            "APP",
            "HMI",
            5,
            "User pressed button: HOME (debug trace)",
        ),
        (
            "ECU1",
            "SYS",
            "BOOT",
            1,
            "FATAL: Watchdog timer expired! Resetting ECU...",
        ),
        ("ECU1", "NAV", "GPS", 4, "Satellite count: 12, HDOP: 0.9"),
        (
            "ECU1",
            "DIAG",
            "OBD",
            4,
            "Coolant temperature: 92°C (normal range)",
        ),
        (
            "ECU1",
            "APP",
            "HMI",
            6,
            "Verbose: rendering frame #12847, delta=16ms",
        ),
        (
            "ECU1",
            "SYS",
            "MEM",
            3,
            "Heap fragmentation warning: 23% fragmented",
        ),
        (
            "ECU2",
            "COM",
            "ETH",
            4,
            "Ethernet link up on eth0: 100Mbps Full-Duplex",
        ),
        (
            "ECU2",
            "COM",
            "ETH",
            2,
            "Ethernet CRC error count exceeded threshold",
        ),
        (
            "ECU2",
            "SEC",
            "AUTH",
            4,
            "Certificate validation passed for service XYZ",
        ),
        (
            "ECU2",
            "SEC",
            "AUTH",
            2,
            "Authentication failed: invalid token from client 10.0.0.5",
        ),
        (
            "ECU2",
            "COM",
            "DOIP",
            4,
            "DoIP connection established with tester 10.0.0.99",
        ),
    ];

    let mut file = std::fs::File::create(&out_path).expect("Failed to create sample.dlt");
    for (ecu, apid, ctid, level, payload) in &messages {
        let msg = build_dlt_message(ecu, apid, ctid, *level, payload);
        file.write_all(&msg).expect("Failed to write message");
    }

    println!("Generated sample DLT file at: {}", out_path.display());
    println!("Total messages: {}", messages.len());
}
