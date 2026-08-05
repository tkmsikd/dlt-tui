use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dlt-tui"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn cli_reports_help_version_and_argument_errors() {
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("USAGE:"));

    let version = run(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        format!("dlt-tui {}", env!("CARGO_PKG_VERSION"))
    );

    let unknown = run(&["--bogus"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown option '--bogus'"));

    let conflict = run(&["--connect", "127.0.0.1:3490", "capture.dlt"]);
    assert_eq!(conflict.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("cannot be combined"));
}
