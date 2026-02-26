use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{env, io, path::PathBuf};

use crate::app::App;

pub mod app;
pub mod explorer;
pub mod fs_reader;
pub mod parser;
pub mod tcp_client;
pub mod ui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse arguments BEFORE entering raw mode so --help and errors
    // print cleanly to the terminal without corruption.
    let args: Vec<String> = env::args().collect();
    let mut connect_addr: Option<String> = None;
    let mut file_path: Option<PathBuf> = None;

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
                println!("    dlt-tui [OPTIONS] [PATH]");
                println!();
                println!("ARGS:");
                println!("    [PATH]    File or directory to open");
                println!();
                println!("OPTIONS:");
                println!("    -c, --connect <HOST:PORT>    Connect to a dlt-daemon TCP socket");
                println!("    -h, --help                   Print help information");
                std::process::exit(0);
            }
            other => {
                file_path = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }

    // Setup terminal (raw mode) — only after argument validation passes
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and initialize
    let mut app = App::new();

    if let Some(addr) = connect_addr {
        app.connect_tcp(&addr);
    } else {
        let init_path = file_path.unwrap_or_else(|| env::current_dir().unwrap_or_default());

        if init_path.is_dir() {
            if let Err(e) = app.load_directory(&init_path) {
                app.error_message = Some(format!("Could not load directory: {}", e));
            }
        } else {
            let parent = init_path.parent().unwrap_or(&init_path);
            if let Err(e) = app.load_directory(parent) {
                app.error_message = Some(format!("Could not load directory: {}", e));
            }
            if app.error_message.is_none()
                && let Err(e) = app.load_file(&init_path)
            {
                app.error_message = Some(format!("Could not load file: {}", e));
            }
        }
    }

    let res = run_app(&mut terminal, app);

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
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
