use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{env, io, path::PathBuf};

use crate::app::App;

pub mod app;
pub mod explorer;
pub mod exporter;
pub mod fs_reader;
pub mod parser;
pub mod tcp_client;
pub mod ui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse arguments BEFORE entering raw mode so --help and errors
    // print cleanly to the terminal without corruption.
    let args: Vec<String> = env::args().collect();
    let mut connect_addr: Option<String> = None;
    let mut file_paths: Vec<PathBuf> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--connect" | "-c" => {
                i += 1;
                if i < args.len() {
                    connect_addr = Some(args[i].clone());
                } else {
                    eprintln!("Error: --connect requires an address (e.g., localhost:3490)");
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                println!("dlt-tui - A fast TUI viewer for Automotive DLT logs");
                println!();
                println!("USAGE:");
                println!("    dlt-tui [OPTIONS] [PATH...]");
                println!();
                println!("ARGS:");
                println!("    [PATH...]  File(s) or directory to open");
                println!();
                println!("OPTIONS:");
                println!("    -c, --connect <HOST:PORT>    Connect to a dlt-daemon TCP socket");
                println!("    -h, --help                   Print help information");
                println!("    -V, --version                Print version information");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("dlt-tui {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => {
                file_paths.push(PathBuf::from(other));
            }
        }
        i += 1;
    }

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
