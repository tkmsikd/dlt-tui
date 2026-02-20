use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{env, io, path::PathBuf};

use crate::app::{App, AppScreen};

pub mod app;
pub mod explorer;
pub mod fs_reader;
pub mod parser;
pub mod ui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let mut app = App::new();

    // determine init path
    let args: Vec<String> = env::args().collect();
    let init_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        env::current_dir()?
    };

    if init_path.is_dir() {
        app.load_directory(&init_path)?;
    } else {
        // If passed a file, attempt to load it right away
        let parent = init_path.parent().unwrap_or(&init_path);
        app.load_directory(parent)?;
        app.load_file(&init_path)?;
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
    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') => app.on_key_q(),
                KeyCode::Char('j') | KeyCode::Down => app.on_down(),
                KeyCode::Char('k') | KeyCode::Up => app.on_up(),
                KeyCode::Enter => {
                    // MVP logic for Enter key (navigate or open)
                    if app.screen == AppScreen::Explorer {
                        if !app.explorer_items.is_empty() {
                            let selected_item = &app.explorer_items[app.explorer_selected_index];
                            if selected_item.is_dir {
                                let path_clone = selected_item.path.clone();
                                let _ = app.load_directory(&path_clone);
                            } else {
                                let path_clone = selected_item.path.clone();
                                let _ = app.load_file(&path_clone);
                            }
                        }
                    } else if app.screen == AppScreen::LogViewer {
                        app.screen = AppScreen::Explorer; // Return to explorer
                    }
                }
                KeyCode::Esc => {
                    if app.screen == AppScreen::LogViewer {
                        app.screen = AppScreen::Explorer;
                    }
                }
                _ => {}
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
