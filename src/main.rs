mod agent;
mod cli;
mod diff;
mod highlight;
mod input;
mod landing;
mod layout;
mod log;
mod note;
mod render;
mod state;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};

use crate::{
    agent::ReviewContext,
    cli::{Cli, Command},
    diff::{DiffFilter, DiffSession, DiffTarget, discover_repo_root},
    input::NavAction,
    state::{App, FocusPane},
};

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let repo_path = cli.repo.clone().unwrap_or_else(|| PathBuf::from("."));
    let direct_diff = match &cli.command {
        Some(Command::Diff(args)) => Some((
            args.diff_target().unwrap_or_else(|error| error.exit()),
            args.diff_filter().unwrap_or_else(|error| error.exit()),
        )),
        None => None,
    };

    let mut terminal = setup_terminal()?;
    let result = match direct_diff {
        Some((diff_target, diff_filter)) => run_app(
            &mut terminal,
            &repo_path,
            &diff_target,
            diff_filter.as_ref(),
        ),
        None => run_landing_flow(&mut terminal, &repo_path),
    };
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
    repo_path: &Path,
    diff_target: &DiffTarget,
    diff_filter: Option<&DiffFilter>,
) -> io::Result<()> {
    let mut diff_load = log::timer("diff_load");
    let session =
        DiffSession::load_from_repo(repo_path, diff_target, diff_filter).unwrap_or_default();

    diff_load.field("files", session.num_files());
    diff_load.field("hunks", session.num_hunks());
    diff_load.field("lines", session.num_lines());

    let repo_root = discover_repo_root(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let mut app =
        App::new_with_review_context(session, ReviewContext::new(repo_root, diff_target.clone()));
    let first_frame_start = Instant::now();
    let mut first_frame_logged = false;

    while app.global.running {
        app.drain_agent_events();
        let viewport_area = terminal.get_frame().area();
        render::ensure_layout(&mut app, viewport_area);
        app.clamp_cursor_row(render::max_cursor_row(&app));
        app.sync_selection_to_cursor();
        render::ensure_cursor_visible(&mut app, viewport_area);

        app.main_pane.scroll = app
            .main_pane
            .scroll
            .min(render::max_scroll(&app, viewport_area));

        if app.global.focus != FocusPane::Files {
            app.sync_sidebar_cursor_to_selected_file();
        }

        terminal.draw(|frame| render::render(frame, &app))?;

        if !first_frame_logged {
            let elapsed_ms = first_frame_start.elapsed().as_millis().to_string();
            let num_rows = app
                .layout
                .as_ref()
                .map(|layout| layout.row_count)
                .unwrap_or(0)
                .to_string();
            let mut fields = vec![("elapsed_ms", elapsed_ms), ("rows", num_rows)];
            if let Some(rss_mb) = log::current_rss_mb() {
                fields.push(("rss_mb", rss_mb));
            }

            log::add_event("first_frame", &fields);
            first_frame_logged = true;
        }

        if let Some(action) = input::poll_and_handle_events(&mut app)? {
            let viewport_area = terminal.get_frame().area();
            render::ensure_layout(&mut app, viewport_area);

            app.clamp_cursor_row(render::max_cursor_row(&app));
            app.main_pane.scroll = app
                .main_pane
                .scroll
                .min(render::max_scroll(&app, viewport_area));

            match action {
                NavAction::RevealSelectedHunk => {
                    render::reveal_selected_hunk(&mut app, viewport_area)
                }
                NavAction::SyncSelectionToScroll => {
                    app.sync_selection_to_cursor();
                    render::ensure_cursor_visible(&mut app, viewport_area);
                }
                NavAction::None => {}
            }

            app.sync_selection_to_cursor();
            render::ensure_cursor_visible(&mut app, viewport_area);

            if app.global.focus != FocusPane::Files {
                app.sync_sidebar_cursor_to_selected_file();
            }
        }
    }

    Ok(())
}

fn run_landing_flow(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    repo_path: &Path,
) -> io::Result<()> {
    let Some(selection) = landing::run_landing_page(terminal, repo_path)? else {
        return Ok(());
    };

    run_app(
        terminal,
        repo_path,
        &selection.target,
        selection.diff_filter.as_ref(),
    )
}
