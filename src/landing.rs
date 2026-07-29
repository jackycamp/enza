use std::{
    collections::HashSet,
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use git2::{BranchType, Repository};
use ratatui::{
    Frame, Terminal,
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout as FrameLayout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::diff::{DiffFilter, DiffSession, DiffTarget};

const FRAME_INTERVAL: Duration = Duration::from_millis(50);
const BASELINE_PARTICLE_COUNT: usize = 32;
const MAX_PARTICLE_COUNT: usize = 1024;

const LOGO: &[&str] = &[
    " ______     __   __     ______     ______    ",
    "/\\  ___\\   /\\ \"-.\\ \\   /\\___  \\   /\\  __ \\   ",
    "\\ \\  __\\   \\ \\ \\-.  \\  \\/_/  /__  \\ \\  __ \\  ",
    " \\ \\_____\\  \\ \\_\\\\\"\\_\\   /\\_____\\  \\ \\_\\ \\_\\ ",
    "  \\/_____/   \\/_/ \\/_/   \\/_____/   \\/_/\\/_/",
];

pub struct LandingSelection {
    pub target: DiffTarget,
    pub diff_filter: Option<DiffFilter>,
}

struct LandingApp {
    suggestions: Vec<LandingSuggestion>,
    selected: usize,
    particles: ParticleField,
}

struct LandingData {
    suggestions: Vec<LandingSuggestion>,
    worktree_stats: DiffStats,
}

struct LandingWorker {
    result_rx: Receiver<LandingData>,
    cancelled: Arc<AtomicBool>,
}

struct SuggestionCollector<'a> {
    suggestions: Vec<LandingSuggestion>,
    seen: HashSet<String>,
    repo_path: &'a Path,
    cancelled: &'a AtomicBool,
}

enum LandingAction {
    Continue,
    Quit,
    Open(LandingSelection),
}

#[derive(Clone)]
struct LandingSuggestion {
    title: String,
    command: String,
    detail: String,
    target: DiffTarget,
    diff_filter: Option<DiffFilter>,
}

#[derive(Clone, Copy, Debug, Default)]
struct DiffStats {
    files: usize,
    hunks: usize,
    additions: usize,
    deletions: usize,
}

struct ParticleField {
    particles: Vec<Particle>,
    rng: TinyRng,
    add_ratio: f32,
    area_size: Option<(u16, u16)>,
}

#[derive(Clone, Copy)]
struct Particle {
    glyph: char,
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    age: f32,
    lifetime: f32,
    pulse_offset: f32,
}

struct TinyRng {
    state: u64,
}

pub fn run_landing_page<B: Backend>(
    terminal: &mut Terminal<B>,
    repo_path: &Path,
) -> io::Result<Option<LandingSelection>> {
    let mut app = LandingApp::new();
    let mut last_tick = Instant::now();

    app.tick(0.0, terminal.get_frame().area());
    terminal.draw(|frame| render(frame, &app))?;

    let worker = LandingWorker::new(repo_path);

    loop {
        if let Some(data) = worker.take_result() {
            app.apply_data(data);
        }

        let now = Instant::now();
        let dt = now.duration_since(last_tick).as_secs_f32().min(0.25);
        last_tick = now;
        app.tick(dt, terminal.get_frame().area());

        terminal.draw(|frame| render(frame, &app))?;

        if event::poll(FRAME_INTERVAL)? {
            match handle_event(&mut app, event::read()?) {
                LandingAction::Continue => {}
                LandingAction::Quit => return Ok(None),
                LandingAction::Open(selection) => return Ok(Some(selection)),
            }
        }
    }
}

impl LandingApp {
    fn new() -> Self {
        Self {
            suggestions: vec![loading_worktree_suggestion()],
            selected: 0,
            particles: ParticleField::new(DiffStats::default()),
        }
    }

    fn apply_data(&mut self, data: LandingData) {
        let selected_command = self
            .suggestions
            .get(self.selected)
            .map(|suggestion| suggestion.command.as_str());
        let selected = selected_command
            .and_then(|command| {
                data.suggestions
                    .iter()
                    .position(|suggestion| suggestion.command == command)
            })
            .unwrap_or(0);

        self.suggestions = data.suggestions;
        self.selected = selected.min(self.suggestions.len().saturating_sub(1));
        self.particles = ParticleField::new(data.worktree_stats);
    }

    fn next(&mut self) {
        if self.suggestions.is_empty() {
            return;
        }

        self.selected = (self.selected + 1) % self.suggestions.len();
    }

    fn previous(&mut self) {
        if self.suggestions.is_empty() {
            return;
        }

        self.selected = if self.selected == 0 {
            self.suggestions.len() - 1
        } else {
            self.selected - 1
        };
    }

    fn select(&self) -> Option<LandingSelection> {
        self.suggestions
            .get(self.selected)
            .map(|suggestion| LandingSelection {
                target: suggestion.target.clone(),
                diff_filter: suggestion.diff_filter.clone(),
            })
    }

    fn tick(&mut self, dt: f32, area: Rect) {
        self.particles.update(dt, area);
    }
}

impl LandingWorker {
    fn new(repo_path: &Path) -> Self {
        let repo_path = repo_path.to_path_buf();
        Self::spawn(move |cancelled| load_landing_data(&repo_path, cancelled))
    }

    fn spawn<F>(load: F) -> Self
    where
        F: FnOnce(&AtomicBool) -> Option<LandingData> + Send + 'static,
    {
        let (result_tx, result_rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);

        thread::spawn(move || {
            let Some(data) = load(&worker_cancelled) else {
                return;
            };
            if !worker_cancelled.load(Ordering::Relaxed) {
                let _ = result_tx.send(data);
            }
        });

        Self {
            result_rx,
            cancelled,
        }
    }

    fn take_result(&self) -> Option<LandingData> {
        self.result_rx.try_recv().ok()
    }
}

impl Drop for LandingWorker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl ParticleField {
    fn new(stats: DiffStats) -> Self {
        let total_changes = stats.additions.saturating_add(stats.deletions);
        let count = particle_count(stats);
        let add_ratio = if total_changes == 0 {
            0.5
        } else {
            stats.additions as f32 / total_changes as f32
        };

        Self {
            particles: vec![Particle::default(); count],
            rng: TinyRng::new(seed_for_launch(stats)),
            add_ratio,
            area_size: None,
        }
    }

    fn update(&mut self, dt: f32, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let area_size = (area.width, area.height);
        if self.area_size != Some(area_size) {
            self.area_size = Some(area_size);
            for particle in &mut self.particles {
                *particle = spawn_particle(&mut self.rng, self.add_ratio, area, true);
            }
            return;
        }

        for particle in &mut self.particles {
            particle.age += dt;
            particle.x += particle.dx * dt;
            particle.y += particle.dy * dt;

            if particle.age >= particle.lifetime || particle.outside(area) {
                *particle = spawn_particle(&mut self.rng, self.add_ratio, area, false);
            }
        }
    }
}

impl Default for Particle {
    fn default() -> Self {
        Self {
            glyph: '+',
            x: -1.0,
            y: -1.0,
            dx: 0.0,
            dy: 0.0,
            age: 1.0,
            lifetime: 0.0,
            pulse_offset: 0.0,
        }
    }
}

impl Particle {
    fn outside(self, area: Rect) -> bool {
        let max_x = area.width as f32 + 2.0;
        let max_y = area.height as f32 + 2.0;
        self.x < -2.0 || self.y < -2.0 || self.x > max_x || self.y > max_y
    }

    fn style(self) -> Style {
        let progress = if self.lifetime <= 0.0 {
            1.0
        } else {
            (self.age / self.lifetime).clamp(0.0, 1.0)
        };
        let fade = (1.0 - (progress - 0.5).abs() * 2.0).clamp(0.0, 1.0);
        let pulse = ((self.age * 5.0 + self.pulse_offset).sin() + 1.0) * 0.5;
        let intensity = (fade * 0.75 + pulse * 0.35).clamp(0.25, 1.0);

        let color = if self.glyph == '+' {
            particle_color((58, 125, 75), (110, 220, 130), intensity)
        } else {
            particle_color((130, 55, 60), (230, 100, 105), intensity)
        };

        Style::default().fg(color)
    }
}

impl TinyRng {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }

    fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }

    fn range_f32(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.next_f32()
    }

    fn chance(&mut self, probability: f32) -> bool {
        self.next_f32() < probability
    }
}

fn load_landing_data(repo_path: &Path, cancelled: &AtomicBool) -> Option<LandingData> {
    let worktree_stats = load_stats(repo_path, &DiffTarget::Worktree, None).unwrap_or_default();
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }

    let suggestions = build_suggestions(repo_path, worktree_stats, cancelled);
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }

    Some(LandingData {
        suggestions,
        worktree_stats,
    })
}

fn build_suggestions(
    repo_path: &Path,
    worktree_stats: DiffStats,
    cancelled: &AtomicBool,
) -> Vec<LandingSuggestion> {
    let mut collector = SuggestionCollector {
        suggestions: Vec::new(),
        seen: HashSet::new(),
        repo_path,
        cancelled,
    };
    push_worktree_suggestion(
        &mut collector.suggestions,
        &mut collector.seen,
        worktree_stats,
    );
    push_changed_suggestion(
        &mut collector,
        "Review staged changes".to_string(),
        "enza diff --cached".to_string(),
        DiffTarget::Cached,
        None,
        "Staged for the next commit",
    );

    if let Ok(repo) = Repository::discover(repo_path) {
        if let Some(upstream) = current_upstream(&repo) {
            push_changed_suggestion(
                &mut collector,
                "Review branch changes since upstream".to_string(),
                format!("enza diff {upstream}...HEAD"),
                DiffTarget::MergeBaseRange {
                    base: upstream.clone(),
                    head: "HEAD".to_string(),
                },
                None,
                "Compared from the merge base with upstream",
            );
            push_changed_suggestion(
                &mut collector,
                "Review unpushed commits".to_string(),
                format!("enza diff {upstream}..HEAD"),
                DiffTarget::Range {
                    base: upstream.clone(),
                    head: "HEAD".to_string(),
                },
                None,
                "Commits on this branch that are not upstream",
            );
            push_changed_suggestion(
                &mut collector,
                "Review incoming upstream changes".to_string(),
                format!("enza diff HEAD..{upstream}"),
                DiffTarget::Range {
                    base: "HEAD".to_string(),
                    head: upstream,
                },
                None,
                "Upstream commits not in this branch",
            );
        }

        for base in ["main", "master"] {
            if revision_exists(&repo, base) {
                push_changed_suggestion(
                    &mut collector,
                    format!("Review branch changes since {base}"),
                    format!("enza diff {base}...HEAD"),
                    DiffTarget::MergeBaseRange {
                        base: base.to_string(),
                        head: "HEAD".to_string(),
                    },
                    None,
                    &format!("Compared from the merge base with {base}"),
                );
            }
        }

        for (base, title, context) in [
            (
                "HEAD~1",
                "Review the last commit",
                "Changes introduced by the most recent commit",
            ),
            (
                "HEAD~5",
                "Review recent commits",
                "Changes across the last five commits",
            ),
        ] {
            if revision_exists(&repo, base) {
                push_changed_suggestion(
                    &mut collector,
                    title.to_string(),
                    format!("enza diff {base}..HEAD"),
                    DiffTarget::Range {
                        base: base.to_string(),
                        head: "HEAD".to_string(),
                    },
                    None,
                    context,
                );
            }
        }
    }

    for (filter, title, context) in [
        (
            "M",
            "Review modified files only",
            "Working tree files changed in place",
        ),
        (
            "A",
            "Review added files only",
            "New or untracked working tree files",
        ),
        (
            "D",
            "Review deleted files only",
            "Working tree files removed from disk",
        ),
    ] {
        let Some(diff_filter) = DiffFilter::parse(filter) else {
            continue;
        };
        push_changed_suggestion(
            &mut collector,
            title.to_string(),
            format!("enza diff --diff-filter {filter}"),
            DiffTarget::Worktree,
            Some(diff_filter),
            context,
        );
    }

    collector.suggestions
}

fn push_worktree_suggestion(
    suggestions: &mut Vec<LandingSuggestion>,
    seen: &mut HashSet<String>,
    stats: DiffStats,
) {
    let command = "enza diff".to_string();
    if !seen.insert(command.clone()) {
        return;
    }

    let detail = if stats.has_changes() {
        format!("Changes not yet staged. {}", stats.summary())
    } else {
        "No working tree changes".to_string()
    };

    suggestions.push(LandingSuggestion {
        title: "Review your working tree".to_string(),
        command,
        detail,
        target: DiffTarget::Worktree,
        diff_filter: None,
    });
}

fn loading_worktree_suggestion() -> LandingSuggestion {
    LandingSuggestion {
        title: "Review your working tree".to_string(),
        command: "enza diff".to_string(),
        detail: "Calculating repository changes...".to_string(),
        target: DiffTarget::Worktree,
        diff_filter: None,
    }
}

fn push_changed_suggestion(
    collector: &mut SuggestionCollector<'_>,
    title: String,
    command: String,
    target: DiffTarget,
    diff_filter: Option<DiffFilter>,
    context: &str,
) {
    if collector.cancelled.load(Ordering::Relaxed) {
        return;
    }

    if !collector.seen.insert(command.clone()) {
        return;
    }

    let Some(stats) = load_stats(collector.repo_path, &target, diff_filter.as_ref()) else {
        return;
    };
    if collector.cancelled.load(Ordering::Relaxed) {
        return;
    }

    if !stats.has_changes() {
        return;
    }

    collector.suggestions.push(LandingSuggestion {
        title,
        command,
        detail: format!("{context}. {}", stats.summary()),
        target,
        diff_filter,
    });
}

fn load_stats(
    repo_path: &Path,
    target: &DiffTarget,
    diff_filter: Option<&DiffFilter>,
) -> Option<DiffStats> {
    let session = DiffSession::load_from_repo(repo_path, target, diff_filter).ok()?;
    Some(stats_for_session(&session))
}

fn stats_for_session(session: &DiffSession) -> DiffStats {
    let mut stats = DiffStats {
        files: session.files.len(),
        ..DiffStats::default()
    };

    for file in &session.files {
        stats.hunks += file.hunks.len();
        let (additions, deletions) = file.change_counts();
        stats.additions += additions;
        stats.deletions += deletions;
    }

    stats
}

fn current_upstream(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }

    let branch_name = head.shorthand()?;
    let branch = repo.find_branch(branch_name, BranchType::Local).ok()?;
    let upstream = branch.upstream().ok()?;
    upstream.name().ok().flatten().map(str::to_string)
}

fn revision_exists(repo: &Repository, revision: &str) -> bool {
    repo.revparse_single(revision).is_ok()
}

impl DiffStats {
    fn has_changes(self) -> bool {
        self.files > 0 || self.hunks > 0 || self.additions > 0 || self.deletions > 0
    }

    fn summary(self) -> String {
        format!(
            "{} {}, +{}, -{}",
            self.files,
            plural(self.files, "file", "files"),
            self.additions,
            self.deletions
        )
    }
}

fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

fn particle_count(stats: DiffStats) -> usize {
    let total_changes = stats.additions.saturating_add(stats.deletions);
    let scaled = (total_changes as f32).sqrt() as usize / 2;
    BASELINE_PARTICLE_COUNT
        .saturating_add(stats.files)
        .saturating_add(scaled)
        .min(MAX_PARTICLE_COUNT)
}

fn seed_for_launch(stats: DiffStats) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);

    now ^ ((stats.files as u64) << 48) ^ ((stats.additions as u64) << 24) ^ (stats.deletions as u64)
}

fn spawn_particle(rng: &mut TinyRng, add_ratio: f32, area: Rect, fresh: bool) -> Particle {
    let shooting = rng.chance(0.14);
    let glyph = if rng.chance(add_ratio) { '+' } else { '-' };
    let lifetime = if shooting {
        rng.range_f32(1.0, 2.4)
    } else {
        rng.range_f32(4.0, 9.0)
    };
    let age = if fresh {
        rng.range_f32(0.0, lifetime)
    } else {
        0.0
    };

    let (x, y, dx, dy) = if shooting {
        (
            rng.range_f32(-12.0, area.width as f32 * 0.7),
            rng.range_f32(0.0, area.height as f32 * 0.75),
            rng.range_f32(7.0, 16.0),
            rng.range_f32(0.6, 2.4),
        )
    } else {
        (
            rng.range_f32(0.0, area.width.saturating_sub(1) as f32),
            rng.range_f32(0.0, area.height.saturating_sub(1) as f32),
            rng.range_f32(-0.7, 0.7),
            rng.range_f32(-0.35, 0.35),
        )
    };

    Particle {
        glyph,
        x,
        y,
        dx,
        dy,
        age,
        lifetime,
        pulse_offset: rng.range_f32(0.0, std::f32::consts::TAU),
    }
}

fn particle_color(low: (u8, u8, u8), high: (u8, u8, u8), intensity: f32) -> Color {
    let mix = |low: u8, high: u8| {
        low.saturating_add(((high.saturating_sub(low)) as f32 * intensity) as u8)
    };

    Color::Rgb(mix(low.0, high.0), mix(low.1, high.1), mix(low.2, high.2))
}

fn handle_event(app: &mut LandingApp, event: Event) -> LandingAction {
    match event {
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            handle_key_event(app, key)
        }
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollDown => {
                app.next();
                LandingAction::Continue
            }
            MouseEventKind::ScrollUp => {
                app.previous();
                LandingAction::Continue
            }
            _ => LandingAction::Continue,
        },
        _ => LandingAction::Continue,
    }
}

fn handle_key_event(app: &mut LandingApp, key: KeyEvent) -> LandingAction {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q') | KeyCode::Esc, _) => LandingAction::Quit,
        (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            LandingAction::Quit
        }
        (KeyCode::Char('j') | KeyCode::Down, _) => {
            app.next();
            LandingAction::Continue
        }
        (KeyCode::Char('k') | KeyCode::Up, _) => {
            app.previous();
            LandingAction::Continue
        }
        (KeyCode::Enter, _) => app
            .select()
            .map(LandingAction::Open)
            .unwrap_or(LandingAction::Continue),
        _ => LandingAction::Continue,
    }
}

fn render(frame: &mut Frame<'_>, app: &LandingApp) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Black)),
        area,
    );

    let content = centered_width(area, 88);
    let suggestion_height = (app.suggestions.len() as u16)
        .saturating_mul(3)
        .saturating_add(2)
        .clamp(4, 14);
    let content_height = (LOGO.len() as u16)
        .saturating_add(1)
        .saturating_add(suggestion_height);
    let top_padding = area.height.saturating_sub(content_height) / 2;
    let foreground_area = Rect {
        x: content.x,
        y: content.y + top_padding,
        width: content.width,
        height: content_height,
    };

    render_particles(frame, area, foreground_area, &app.particles);

    let chunks = FrameLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_padding),
            Constraint::Length(LOGO.len() as u16),
            Constraint::Length(1),
            Constraint::Length(suggestion_height),
            Constraint::Min(1),
        ])
        .split(content);

    render_logo(frame, chunks[1]);
    render_suggestions(frame, chunks[3], app);
}

fn render_particles(frame: &mut Frame<'_>, area: Rect, excluded: Rect, particles: &ParticleField) {
    for particle in &particles.particles {
        if particle.lifetime <= 0.0 {
            continue;
        }

        render_particle_cell(
            frame,
            area,
            excluded,
            particle.x,
            particle.y,
            particle.glyph,
            particle.style(),
        );
    }
}

fn render_particle_cell(
    frame: &mut Frame<'_>,
    area: Rect,
    excluded: Rect,
    x: f32,
    y: f32,
    glyph: char,
    style: Style,
) {
    let x = x.round();
    let y = y.round();
    if x < 0.0 || y < 0.0 || x >= area.width as f32 || y >= area.height as f32 {
        return;
    }

    let x = area.x + x as u16;
    let y = area.y + y as u16;
    if rect_contains(excluded, x, y) {
        return;
    }

    frame.render_widget(
        Paragraph::new(glyph.to_string()).style(style),
        Rect {
            x,
            y,
            width: 1,
            height: 1,
        },
    );
}

fn rect_contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn render_logo(frame: &mut Frame<'_>, area: Rect) {
    let lines = LOGO
        .iter()
        .map(|line| {
            Line::from(Span::styled(
                *line,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ))
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(Style::default().bg(Color::Black)),
        area,
    );
}

fn render_suggestions(frame: &mut Frame<'_>, area: Rect, app: &LandingApp) {
    let items = app
        .suggestions
        .iter()
        .map(|suggestion| {
            ListItem::new(vec![
                Line::from(Span::styled(
                    suggestion.title.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    suggestion.command.clone(),
                    Style::default().fg(Color::Gray),
                )),
                Line::from(Span::styled(
                    suggestion.detail.clone(),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(app.selected));

    frame.render_stateful_widget(
        List::new(items)
            .style(Style::default().bg(Color::Black))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().bg(Color::Black))
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .highlight_symbol("> ")
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
        area,
        &mut state,
    );
}

fn centered_width(area: Rect, width: u16) -> Rect {
    let width = width.min(area.width);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y,
        width,
        height: area.height,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui::{Terminal, backend::TestBackend};

    use super::{
        BASELINE_PARTICLE_COUNT, DiffStats, LandingApp, LandingData, LandingSuggestion,
        LandingWorker, MAX_PARTICLE_COUNT, particle_count, render,
    };
    use crate::diff::DiffTarget;

    #[test]
    fn landing_starts_with_an_openable_worktree_while_loading() {
        let app = LandingApp::new();

        assert_eq!(app.suggestions.len(), 1);
        assert_eq!(app.suggestions[0].command, "enza diff");
        assert_eq!(
            app.suggestions[0].detail,
            "Calculating repository changes..."
        );
        assert_eq!(app.select().unwrap().target, DiffTarget::Worktree);
    }

    #[test]
    fn initial_frame_renders_the_loading_state() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = LandingApp::new();

        terminal.draw(|frame| render(frame, &app)).unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Calculating repository changes..."));
    }

    #[test]
    fn loaded_suggestions_replace_the_placeholder_and_preserve_selection() {
        let mut app = LandingApp::new();
        app.apply_data(LandingData {
            suggestions: vec![
                suggestion("enza diff --cached", DiffTarget::Cached),
                suggestion("enza diff", DiffTarget::Worktree),
            ],
            worktree_stats: DiffStats {
                files: 2,
                additions: 16,
                ..DiffStats::default()
            },
        });

        assert_eq!(app.selected, 1);
        assert_eq!(app.select().unwrap().target, DiffTarget::Worktree);
        assert_eq!(app.particles.particles.len(), BASELINE_PARTICLE_COUNT + 4);
    }

    #[test]
    fn landing_worker_publishes_loaded_data() {
        let worker = LandingWorker::spawn(|_| {
            Some(LandingData {
                suggestions: vec![suggestion("enza diff", DiffTarget::Worktree)],
                worktree_stats: DiffStats::default(),
            })
        });

        let data = worker
            .result_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert_eq!(data.suggestions.len(), 1);
        assert_eq!(data.suggestions[0].command, "enza diff");
    }

    #[test]
    fn particle_count_keeps_a_baseline_for_clean_worktrees() {
        assert_eq!(
            particle_count(DiffStats::default()),
            BASELINE_PARTICLE_COUNT
        );
    }

    #[test]
    fn particle_count_scales_with_changes_up_to_the_cap() {
        assert_eq!(
            particle_count(DiffStats {
                files: 2,
                additions: 16,
                deletions: 0,
                ..DiffStats::default()
            }),
            BASELINE_PARTICLE_COUNT + 4
        );

        assert_eq!(
            particle_count(DiffStats {
                files: MAX_PARTICLE_COUNT.saturating_mul(2),
                additions: 0,
                deletions: 0,
                ..DiffStats::default()
            }),
            MAX_PARTICLE_COUNT
        );
    }

    fn suggestion(command: &str, target: DiffTarget) -> LandingSuggestion {
        LandingSuggestion {
            title: "Review changes".to_string(),
            command: command.to_string(),
            detail: "Details".to_string(),
            target,
            diff_filter: None,
        }
    }
}
