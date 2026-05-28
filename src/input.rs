use std::{io, time::Duration};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

use crate::{
    render,
    state::{App, FocusPane},
};

pub enum NavAction {
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

pub fn poll_and_handle_events(app: &mut App) -> io::Result<Option<NavAction>> {
    if !event::poll(Duration::from_millis(16))? {
        return Ok(None);
    }

    let mut action = NavAction::None;
    loop {
        action = action.merge(handle_event(app, event::read()?));
        if !event::poll(Duration::from_millis(0))? {
            break;
        }
    }

    Ok(Some(action))
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
        (KeyCode::Char('j') | KeyCode::Down, _) => match app.global.focus {
            FocusPane::Main => {
                app.move_cursor_down(1, render::max_cursor_row(app));
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => {
                app.file_cursor_down();
                NavAction::None
            }
        },
        (KeyCode::Char('k') | KeyCode::Up, _) => match app.global.focus {
            FocusPane::Main => {
                app.move_cursor_up(1);
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => {
                app.file_cursor_up();
                NavAction::None
            }
        },
        (KeyCode::Char(']'), _) => match app.global.focus {
            FocusPane::Main => {
                app.next_hunk();
                NavAction::RevealSelectedHunk
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('['), _) => match app.global.focus {
            FocusPane::Main => {
                app.previous_hunk();
                NavAction::RevealSelectedHunk
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => match app.global.focus {
            FocusPane::Main => {
                app.move_cursor_down(10, render::max_cursor_row(app));
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => match app.global.focus {
            FocusPane::Main => {
                app.move_cursor_up(10);
                NavAction::SyncSelectionToScroll
            }
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Left, _) => match app.global.focus {
            FocusPane::Files => {
                app.collapse_sidebar_directory();
                NavAction::None
            }
            FocusPane::Main => NavAction::None,
        },
        (KeyCode::Right, _) => match app.global.focus {
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
        (KeyCode::Char('D'), _) => {
            app.toggle_debug_pane();
            NavAction::None
        }
        (KeyCode::Char('n'), _) => match app.global.focus {
            FocusPane::Main => NavAction::PromptForNote,
            FocusPane::Files => NavAction::None,
        },
        (KeyCode::Char('v'), _) => match app.global.focus {
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
        (KeyCode::Enter, _) => match app.global.focus {
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
            app.move_cursor_down(WHEEL_SCROLL_LINES as usize, render::max_cursor_row(app));
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
