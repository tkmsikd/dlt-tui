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
        if let Err(e) = app.load_directory(&init_path) {
            app.error_message = Some(format!("Could not load directory: {}", e));
        }
    } else {
        // If passed a file, attempt to load it right away
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

        if crossterm::event::poll(tick_rate)?
            && let Event::Key(key) = event::read()?
        {
            // Dismiss error on any key press
            if app.error_message.is_some() {
                app.error_message = None;
                continue;
            }

            if let Some(mode) = app.filter_input_mode.clone() {
                match key.code {
                    KeyCode::Enter => {
                        app.filter_input_mode = None;

                        let input = if app.filter_input.is_empty() {
                            None
                        } else {
                            Some(app.filter_input.clone())
                        };

                        match mode {
                            crate::app::FilterInputMode::Text => app.filter.text = input,
                            crate::app::FilterInputMode::AppId => app.filter.app_id = input,
                            crate::app::FilterInputMode::CtxId => app.filter.ctx_id = input,
                            crate::app::FilterInputMode::MinLevel => {
                                app.filter.min_level =
                                    match app.filter_input.to_lowercase().as_str() {
                                        "f" | "fatal" => Some(crate::parser::LogLevel::Fatal),
                                        "e" | "error" => Some(crate::parser::LogLevel::Error),
                                        "w" | "warn" => Some(crate::parser::LogLevel::Warn),
                                        "i" | "info" => Some(crate::parser::LogLevel::Info),
                                        "d" | "debug" => Some(crate::parser::LogLevel::Debug),
                                        "v" | "verbose" => Some(crate::parser::LogLevel::Verbose),
                                        _ => None,
                                    };
                            }
                        }
                        app.apply_filter();
                    }
                    KeyCode::Esc => {
                        app.filter_input_mode = None;
                        app.filter_input.clear();
                        match mode {
                            crate::app::FilterInputMode::Text => app.filter.text = None,
                            crate::app::FilterInputMode::AppId => app.filter.app_id = None,
                            crate::app::FilterInputMode::CtxId => app.filter.ctx_id = None,
                            crate::app::FilterInputMode::MinLevel => app.filter.min_level = None,
                        }

                        app.apply_filter();
                    }
                    KeyCode::Char(c) => {
                        app.filter_input.push(c);
                    }
                    KeyCode::Backspace => {
                        app.filter_input.pop();
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') => match app.screen {
                        AppScreen::Explorer => app.on_key_q(),
                        AppScreen::LogViewer => app.screen = AppScreen::Explorer,
                        AppScreen::LogDetail => app.screen = AppScreen::LogViewer,
                    },
                    KeyCode::Char('j') | KeyCode::Down => app.on_down(),
                    KeyCode::Char('k') | KeyCode::Up => app.on_up(),
                    KeyCode::Char('g') | KeyCode::Home => app.on_home(),
                    KeyCode::Char('G') | KeyCode::End => app.on_end(),
                    KeyCode::Char('/') => {
                        if app.screen == AppScreen::LogViewer {
                            app.filter_input_mode = Some(crate::app::FilterInputMode::Text);
                            app.filter_input.clear();
                        }
                    }
                    KeyCode::Char('l') => {
                        if app.screen == AppScreen::LogViewer {
                            app.filter_input_mode = Some(crate::app::FilterInputMode::MinLevel);
                            app.filter_input.clear();
                        }
                    }
                    KeyCode::Char('a') => {
                        if app.screen == AppScreen::LogViewer {
                            app.filter_input_mode = Some(crate::app::FilterInputMode::AppId);
                            app.filter_input.clear();
                        }
                    }
                    KeyCode::Char('c') => {
                        if app.screen == AppScreen::LogViewer {
                            app.filter_input_mode = Some(crate::app::FilterInputMode::CtxId);
                            app.filter_input.clear();
                        }
                    }
                    KeyCode::Char('C') => {
                        if app.screen == AppScreen::LogViewer {
                            app.filter = crate::app::Filter::default();
                            app.apply_filter();
                        }
                    }
                    KeyCode::Enter => {
                        // MVP logic for Enter key (navigate or open)
                        if app.screen == AppScreen::Explorer {
                            if !app.explorer_items.is_empty() {
                                let selected_item =
                                    &app.explorer_items[app.explorer_selected_index];
                                if selected_item.is_dir {
                                    let path_clone = selected_item.path.clone();
                                    if let Err(e) = app.load_directory(&path_clone) {
                                        app.error_message =
                                            Some(format!("Could not open directory: {}", e));
                                    }
                                } else {
                                    let path_clone = selected_item.path.clone();
                                    if let Err(e) = app.load_file(&path_clone) {
                                        app.error_message =
                                            Some(format!("Could not open file: {}", e));
                                    }
                                }
                            }
                        } else if app.screen == AppScreen::LogViewer
                            && !app.filtered_log_indices.is_empty()
                        {
                            app.screen = AppScreen::LogDetail;
                        }
                    }
                    KeyCode::Esc => {
                        if app.screen == AppScreen::LogViewer {
                            app.screen = AppScreen::Explorer;
                        } else if app.screen == AppScreen::LogDetail {
                            app.screen = AppScreen::LogViewer;
                        }
                    }
                    _ => {}
                }
            }
        } // Close if crossterm::event::poll !

        app.on_tick();

        if app.should_quit {
            return Ok(());
        }
    }
}
