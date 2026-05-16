mod app;
mod cli;
mod diff;
mod highlight;
mod ui;

use std::{io, time::Duration};

use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};

use crate::{
    app::{App, FocusPane},
    cli::Cli,
    diff::DiffSession,
};

fn main() -> io::Result<()> {
    let _cli = Cli::parse();
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let session = DiffSession::load_from_repo(".").unwrap_or_default();
    let mut app = App::new(session);

    while app.running {
        let viewport_area = terminal.get_frame().area();
        app.scroll = app.scroll.min(ui::max_scroll(&app, viewport_area));
        ui::sync_selection_to_scroll(&mut app);
        if app.focus != FocusPane::Files {
            app.sync_sidebar_cursor_to_selected_file();
        }
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(Duration::from_millis(16))? {
            let mut latest_key = None;

            loop {
                if let Event::Key(key) = event::read()?
                    && key.kind == KeyEventKind::Press
                {
                    latest_key = Some(key);
                }

                if !event::poll(Duration::from_millis(0))? {
                    break;
                }
            }

            let Some(key) = latest_key else {
                continue;
            };

            let action = handle_key_event(&mut app, key);
            let viewport_area = terminal.get_frame().area();
            app.scroll = app.scroll.min(ui::max_scroll(&app, viewport_area));
            match action {
                NavAction::RevealSelectedHunk => ui::reveal_selected_hunk(&mut app, viewport_area),
                NavAction::SyncSelectionToScroll => ui::sync_selection_to_scroll(&mut app),
                NavAction::None => {}
            }
            if app.focus != FocusPane::Files {
                app.sync_sidebar_cursor_to_selected_file();
            }
        }
    }

    Ok(())
}

enum NavAction {
    None,
    RevealSelectedHunk,
    SyncSelectionToScroll,
}

fn handle_key_event(app: &mut App, key: KeyEvent) -> NavAction {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) => {
            app.quit();
            NavAction::None
        }
        (KeyCode::Char('j') | KeyCode::Down, _) => match app.focus {
            FocusPane::Main => {
                app.scroll_down(1);
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => {
                app.file_cursor_down();
                NavAction::None
            }
        },
        (KeyCode::Char('k') | KeyCode::Up, _) => match app.focus {
            FocusPane::Main => {
                app.scroll_up(1);
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => {
                app.file_cursor_up();
                NavAction::None
            }
        },
        (KeyCode::Char(']'), _) => match app.focus {
            FocusPane::Main => {
                app.next_hunk();
                NavAction::RevealSelectedHunk
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('['), _) => match app.focus {
            FocusPane::Main => {
                app.previous_hunk();
                NavAction::RevealSelectedHunk
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => match app.focus {
            FocusPane::Main => {
                app.scroll_down(10);
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => match app.focus {
            FocusPane::Main => {
                app.scroll_up(10);
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Left, _) => match app.focus {
            FocusPane::Files => {
                app.collapse_sidebar_directory();
                NavAction::None
            }
            FocusPane::Main => NavAction::None,
        },
        (KeyCode::Right, _) => match app.focus {
            FocusPane::Files => {
                app.expand_sidebar_directory();
                NavAction::None
            }
            FocusPane::Main => NavAction::None,
        },
        (KeyCode::Tab, KeyModifiers::SHIFT) | (KeyCode::BackTab, _) => {
            app.focus_previous();
            NavAction::None
        }
        (KeyCode::Tab, _) => {
            app.focus_next();
            NavAction::None
        }
        (KeyCode::Char('b'), _) => {
            app.toggle_sidebar();
            NavAction::RevealSelectedHunk
        }
        (KeyCode::Enter, _) => match app.focus {
            FocusPane::Files => {
                app.jump_to_file_cursor();
                NavAction::RevealSelectedHunk
            }
            FocusPane::Main => NavAction::None,
        },
        (KeyCode::Char('m'), _) => {
            app.toggle_mode();
            NavAction::RevealSelectedHunk
        }
        _ => NavAction::None,
    }
}
