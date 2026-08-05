use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    env,
    ffi::{OsStr, OsString},
    io,
    net::SocketAddr,
    path::PathBuf,
};

use crate::app::App;

pub mod app;
pub mod explorer;
pub mod exporter;
pub mod fs_reader;
pub mod parser;
pub mod tcp_client;
pub mod ui;

#[cfg(test)]
mod use_case_tests;

#[derive(Debug, PartialEq)]
struct CliOptions {
    connect_addr: Option<String>,
    file_paths: Vec<PathBuf>,
}

#[derive(Debug, PartialEq)]
enum CliCommand {
    Run(CliOptions),
    Help,
    Version,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse arguments BEFORE entering raw mode so --help and errors
    // print cleanly to the terminal without corruption.
    let cli = match parse_cli_args(env::args_os().skip(1)) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("Error: {error}");
            eprintln!("Try 'dlt-tui --help' for more information.");
            std::process::exit(2);
        }
    };

    let (connect_addr, mut file_paths) = match cli {
        CliCommand::Run(options) => (options.connect_addr, options.file_paths),
        CliCommand::Help => {
            print_help();
            return Ok(());
        }
        CliCommand::Version => {
            println!("dlt-tui {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    };

    // Restore the terminal before the default panic handler prints, so the
    // message is not swallowed by the alternate screen.
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_panic(info);
    }));

    // Setup terminal (raw mode) — only after argument validation passes
    let mut cleanup = TerminalCleanup::default();
    enable_raw_mode()?;
    cleanup.raw_mode = true;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    cleanup.alt_screen = true;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and initialize
    let mut app = App::new();

    if let Some(addr) = connect_addr {
        app.connect_tcp(&addr);
    } else {
        if file_paths.is_empty() {
            file_paths.push(env::current_dir().unwrap_or_default());
        }

        if file_paths.len() == 1 && file_paths[0].is_dir() {
            let dir_path = &file_paths[0];
            if let Err(e) = app.load_directory(dir_path) {
                app.error_message = Some(format!("Could not load directory: {}", e));
            }
        } else {
            // Load file(s). Set explorer to the parent of the first file.
            let first_path = &file_paths[0];
            let parent = first_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| env::current_dir().unwrap_or_default());
            if let Err(e) = app.load_directory(&parent) {
                app.error_message = Some(format!("Could not load directory: {}", e));
            }
            if app.error_message.is_none()
                && let Err(e) = app.load_files(file_paths)
            {
                app.error_message = Some(format!("Could not load file(s): {}", e));
            }
        }
    }

    let res = run_app(&mut terminal, app);

    // restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    cleanup.disarm();

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

fn parse_cli_args(args: impl IntoIterator<Item = OsString>) -> Result<CliCommand, String> {
    let mut connect_addr: Option<String> = None;
    let mut file_paths: Vec<PathBuf> = Vec::new();
    let mut positional_only = false;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if positional_only {
            file_paths.push(PathBuf::from(arg));
            continue;
        }

        if arg == OsStr::new("--") {
            positional_only = true;
        } else if arg == OsStr::new("--connect") || arg == OsStr::new("-c") {
            if connect_addr.is_some() {
                return Err("--connect may only be specified once".to_string());
            }

            let address = args.next().ok_or_else(|| {
                "--connect requires an address (for example, localhost:3490)".to_string()
            })?;
            if address.to_string_lossy().starts_with('-') {
                return Err(
                    "--connect requires an address (for example, localhost:3490)".to_string(),
                );
            }
            let address = address
                .into_string()
                .map_err(|_| "--connect address must be valid UTF-8".to_string())?;
            if !valid_connect_address(&address) {
                return Err(
                    "--connect requires HOST:PORT (for example, localhost:3490)".to_string()
                );
            }
            connect_addr = Some(address);
        } else if arg == OsStr::new("--help") || arg == OsStr::new("-h") {
            return Ok(CliCommand::Help);
        } else if arg == OsStr::new("--version") || arg == OsStr::new("-V") {
            return Ok(CliCommand::Version);
        } else if arg.to_string_lossy().starts_with('-') && arg != OsStr::new("-") {
            return Err(format!("unknown option '{}'", arg.to_string_lossy()));
        } else {
            file_paths.push(PathBuf::from(arg));
        }
    }

    if connect_addr.is_some() && !file_paths.is_empty() {
        return Err("--connect cannot be combined with file or directory paths".to_string());
    }

    Ok(CliCommand::Run(CliOptions {
        connect_addr,
        file_paths,
    }))
}

fn valid_connect_address(address: &str) -> bool {
    if address.parse::<SocketAddr>().is_ok() {
        return true;
    }

    address
        .rsplit_once(':')
        .is_some_and(|(host, port)| valid_hostname(host) && port.parse::<u16>().is_ok())
}

fn valid_hostname(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        })
}

fn print_help() {
    println!("dlt-tui - A fast TUI viewer for Automotive DLT logs");
    println!();
    println!("USAGE:");
    println!("    dlt-tui [OPTIONS] [--] [PATH...]");
    println!();
    println!("ARGS:");
    println!("    [PATH...]  File(s) or directory to open");
    println!();
    println!("OPTIONS:");
    println!("    -c, --connect <HOST:PORT>    Connect to a dlt-daemon TCP socket");
    println!("    -h, --help                   Print help information");
    println!("    -V, --version                Print version information");
    println!("    --                           Treat remaining arguments as paths");
}

#[derive(Default)]
struct TerminalCleanup {
    raw_mode: bool,
    alt_screen: bool,
}

impl TerminalCleanup {
    fn disarm(&mut self) {
        self.raw_mode = false;
        self.alt_screen = false;
    }
}

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        if self.raw_mode {
            let _ = disable_raw_mode();
        }
        if self.alt_screen {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
        }
    }
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> io::Result<()> {
    let tick_rate = std::time::Duration::from_millis(50);
    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        let page_size = terminal.size()?.height.saturating_sub(7) as usize;

        if crossterm::event::poll(tick_rate)?
            && let Event::Key(key) = event::read()?
        {
            app.handle_key(key, page_size);
        }

        app.on_tick();

        if app.should_quit {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<CliCommand, String> {
        parse_cli_args(args.iter().map(OsString::from))
    }

    #[test]
    fn cli_parses_file_paths_and_connect_mode() {
        assert_eq!(
            parse(&["first.dlt", "second.dlt.gz"]),
            Ok(CliCommand::Run(CliOptions {
                connect_addr: None,
                file_paths: vec![PathBuf::from("first.dlt"), PathBuf::from("second.dlt.gz")],
            }))
        );
        assert_eq!(
            parse(&["--connect", "localhost:3490"]),
            Ok(CliCommand::Run(CliOptions {
                connect_addr: Some("localhost:3490".to_string()),
                file_paths: Vec::new(),
            }))
        );
        assert_eq!(
            parse(&["--connect", "[::1]:3490"]),
            Ok(CliCommand::Run(CliOptions {
                connect_addr: Some("[::1]:3490".to_string()),
                file_paths: Vec::new(),
            }))
        );
    }

    #[test]
    fn cli_rejects_unknown_and_incomplete_options() {
        assert_eq!(
            parse(&["--bogus"]),
            Err("unknown option '--bogus'".to_string())
        );
        assert!(parse(&["-c"]).unwrap_err().contains("requires an address"));
        assert!(
            parse(&["-c", "--help"])
                .unwrap_err()
                .contains("requires an address")
        );
        for address in [
            "",
            "localhost",
            "localhost:not-a-port",
            ":3490",
            "foo bar:3490",
            "/tmp/socket:3490",
        ] {
            assert!(
                parse(&["-c", address])
                    .unwrap_err()
                    .contains("requires HOST:PORT")
            );
        }
    }

    #[test]
    fn cli_rejects_conflicting_connect_inputs() {
        assert_eq!(
            parse(&["-c", "localhost:3490", "capture.dlt"]),
            Err("--connect cannot be combined with file or directory paths".to_string())
        );
        assert_eq!(
            parse(&["-c", "localhost:3490", "-c", "localhost:3491"]),
            Err("--connect may only be specified once".to_string())
        );
    }

    #[test]
    fn cli_double_dash_preserves_option_like_paths() {
        assert_eq!(
            parse(&["--", "--help", "-trace.dlt"]),
            Ok(CliCommand::Run(CliOptions {
                connect_addr: None,
                file_paths: vec![PathBuf::from("--help"), PathBuf::from("-trace.dlt")],
            }))
        );
    }

    #[cfg(unix)]
    #[test]
    fn cli_preserves_non_utf8_file_paths() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let path = OsString::from_vec(b"capture-\xFF.dlt".to_vec());
        let command = parse_cli_args([path]).unwrap();
        let CliCommand::Run(options) = command else {
            panic!("expected run command");
        };

        assert_eq!(
            options.file_paths[0].as_os_str().as_bytes(),
            b"capture-\xFF.dlt"
        );
    }
}
