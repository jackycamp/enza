mod app;
mod cli;
mod diff;
mod highlight;
mod notes;
mod render_cache;
mod ui;

use std::{io, time::Duration};

use clap::Parser;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEvent, MouseEventKind,
    },
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
    let cli = Cli::parse();
    let diff_target = cli.diff_target().unwrap_or_else(|error| error.exit());
    let diff_filter = cli.diff_filter().unwrap_or_else(|error| error.exit());
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal, &cli, &diff_target, diff_filter.as_ref());
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cli: &Cli,
    diff_target: &crate::diff::DiffTarget,
    diff_filter: Option<&crate::diff::DiffFilter>,
) -> io::Result<()> {
    let repo_path = cli.repo.as_deref().unwrap_or(std::path::Path::new("."));
    let session =
        DiffSession::load_from_repo(repo_path, diff_target, diff_filter).unwrap_or_default();
    let mut app = App::new(session);

    while app.running {
        let viewport_area = terminal.get_frame().area();
        ui::ensure_render_session(&mut app, viewport_area);
        app.clamp_cursor_row(ui::max_cursor_row(&app));
        app.sync_selection_to_cursor();
        ui::ensure_cursor_visible(&mut app, viewport_area);
        app.scroll = app.scroll.min(ui::max_scroll(&app, viewport_area));
        if app.focus != FocusPane::Files {
            app.sync_sidebar_cursor_to_selected_file();
        }
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(Duration::from_millis(16))? {
            let mut action = NavAction::None;

            loop {
                action = action.merge(handle_event(&mut app, event::read()?));

                if !event::poll(Duration::from_millis(0))? {
                    break;
                }
            }

            let viewport_area = terminal.get_frame().area();
            ui::ensure_render_session(&mut app, viewport_area);
            app.clamp_cursor_row(ui::max_cursor_row(&app));
            app.scroll = app.scroll.min(ui::max_scroll(&app, viewport_area));
            match action {
                NavAction::RevealSelectedHunk => ui::reveal_selected_hunk(&mut app, viewport_area),
                NavAction::PromptForNote => app.start_note_input(),
                NavAction::SyncSelectionToScroll => {
                    app.sync_selection_to_cursor();
                    ui::ensure_cursor_visible(&mut app, viewport_area);
                }
                NavAction::None => {}
            }
            app.sync_selection_to_cursor();
            ui::ensure_cursor_visible(&mut app, viewport_area);
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
    PromptForNote,
    SyncSelectionToScroll,
}

impl NavAction {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (_, Self::PromptForNote) | (Self::PromptForNote, _) => Self::PromptForNote,
            (_, Self::RevealSelectedHunk) | (Self::RevealSelectedHunk, _) => {
                Self::RevealSelectedHunk
            }
            (_, Self::SyncSelectionToScroll) | (Self::SyncSelectionToScroll, _) => {
                Self::SyncSelectionToScroll
            }
            _ => Self::None,
        }
    }
}

fn handle_event(app: &mut App, event: Event) -> NavAction {
    match event {
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            handle_key_event(app, key)
        }
        Event::Mouse(mouse) => handle_mouse_event(app, mouse),
        _ => NavAction::None,
    }
}

fn handle_key_event(app: &mut App, key: KeyEvent) -> NavAction {
    if app.note_input_active() {
        return handle_note_input_key_event(app, key);
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) => {
            app.quit();
            NavAction::None
        }
        (KeyCode::Char('j') | KeyCode::Down, _) => match app.focus {
            FocusPane::Main => {
                app.move_cursor_down(1, ui::max_cursor_row(app));
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => {
                app.file_cursor_down();
                NavAction::None
            }
        },
        (KeyCode::Char('k') | KeyCode::Up, _) => match app.focus {
            FocusPane::Main => {
                app.move_cursor_up(1);
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
                app.move_cursor_down(10, ui::max_cursor_row(app));
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => match app.focus {
            FocusPane::Main => {
                app.move_cursor_up(10);
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
        (KeyCode::Char('n'), _) => match app.focus {
            FocusPane::Main => NavAction::PromptForNote,
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('v'), _) => match app.focus {
            FocusPane::Main => {
                app.toggle_selection_anchor();
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Esc, _) => {
            app.clear_selection();
            NavAction::None
        }
        (KeyCode::Enter, _) => match app.focus {
            FocusPane::Files => {
                app.jump_to_file_cursor();
                NavAction::RevealSelectedHunk
            }
            FocusPane::Main => {
                app.toggle_current_note_expanded();
                NavAction::SyncSelectionToScroll
            }
        },
        (KeyCode::Char('m'), _) => {
            app.toggle_mode();
            NavAction::RevealSelectedHunk
        }
        _ => NavAction::None,
    }
}

fn handle_mouse_event(app: &mut App, mouse: MouseEvent) -> NavAction {
    const WHEEL_SCROLL_LINES: u16 = 3;

    match mouse.kind {
        MouseEventKind::ScrollDown => {
            app.move_cursor_down(WHEEL_SCROLL_LINES as usize, ui::max_cursor_row(app));
            NavAction::SyncSelectionToScroll
        }
        MouseEventKind::ScrollUp => {
            app.move_cursor_up(WHEEL_SCROLL_LINES as usize);
            NavAction::SyncSelectionToScroll
        }
        _ => NavAction::None,
    }
}

fn handle_note_input_key_event(app: &mut App, key: KeyEvent) -> NavAction {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => {
            app.cancel_note_input();
            NavAction::None
        }
        (KeyCode::Enter, _) => {
            app.submit_note_input();
            NavAction::None
        }
        (KeyCode::Backspace, _) => {
            app.backspace_note_text();
            NavAction::None
        }
        (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            app.insert_note_text(&ch.to_string());
            NavAction::None
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.cancel_note_input();
            NavAction::None
        }
        _ => NavAction::None,
    }
}
