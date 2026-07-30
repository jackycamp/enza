use std::{
    io,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui::{
    Frame, Terminal,
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout as FrameLayout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use super::loader::{
    DiffLoadWorker, LandingData, LandingSuggestion, LandingWorker, LoadedDiff,
    loading_worktree_suggestion,
};
use crate::diff::{DiffFilter, DiffStats, DiffTarget};

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

struct LandingApp {
    suggestions: Vec<LandingSuggestion>,
    selected: usize,
    particles: ParticleField,
}

struct LandingSelection {
    target: DiffTarget,
    diff_filter: Option<DiffFilter>,
}

enum LandingPhase {
    Discovering(LandingWorker),
    Opening(DiffLoadWorker),
}

enum DiscoveringAction {
    Continue,
    Quit,
    Open(LandingSelection),
}

enum OpeningAction {
    Continue,
    Quit,
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
) -> io::Result<Option<LoadedDiff>> {
    let mut app = LandingApp::new();
    let mut last_tick = Instant::now();

    app.tick(0.0, terminal.get_frame().area());
    terminal.draw(|frame| render(frame, &app))?;

    let mut phase = LandingPhase::Discovering(LandingWorker::new(repo_path));

    loop {
        match &phase {
            LandingPhase::Discovering(worker) => {
                if let Some(data) = worker.take_result() {
                    app.apply_data(data);
                }
            }
            LandingPhase::Opening(worker) => {
                if let Some(loaded) = worker.take_result() {
                    return Ok(Some(loaded));
                }
            }
        }

        let now = Instant::now();
        let dt = now.duration_since(last_tick).as_secs_f32().min(0.25);
        last_tick = now;
        app.tick(dt, terminal.get_frame().area());

        terminal.draw(|frame| render(frame, &app))?;

        if event::poll(FRAME_INTERVAL)? {
            let event = event::read()?;
            phase = match phase {
                LandingPhase::Discovering(worker) => {
                    match handle_discovering_event(&mut app, event) {
                        DiscoveringAction::Continue => LandingPhase::Discovering(worker),
                        DiscoveringAction::Quit => return Ok(None),
                        DiscoveringAction::Open(selection) => {
                            app.show_opening();
                            LandingPhase::Opening(worker.load_diff(
                                repo_path,
                                selection.target,
                                selection.diff_filter,
                            ))
                        }
                    }
                }
                LandingPhase::Opening(worker) => match handle_opening_event(event) {
                    OpeningAction::Continue => LandingPhase::Opening(worker),
                    OpeningAction::Quit => return Ok(None),
                },
            };
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

    fn show_opening(&mut self) {
        if let Some(suggestion) = self.suggestions.get_mut(self.selected) {
            suggestion.detail = "Opening diff...".to_string();
        }
    }

    fn tick(&mut self, dt: f32, area: Rect) {
        self.particles.update(dt, area);
    }
}

fn handle_discovering_event(app: &mut LandingApp, event: Event) -> DiscoveringAction {
    match event {
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            handle_discovering_key_event(app, key)
        }
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollDown => {
                app.next();
                DiscoveringAction::Continue
            }
            MouseEventKind::ScrollUp => {
                app.previous();
                DiscoveringAction::Continue
            }
            _ => DiscoveringAction::Continue,
        },
        _ => DiscoveringAction::Continue,
    }
}

fn handle_discovering_key_event(app: &mut LandingApp, key: KeyEvent) -> DiscoveringAction {
    if is_quit_key(key) {
        return DiscoveringAction::Quit;
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.next();
            DiscoveringAction::Continue
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.previous();
            DiscoveringAction::Continue
        }
        KeyCode::Enter => app
            .select()
            .map(DiscoveringAction::Open)
            .unwrap_or(DiscoveringAction::Continue),
        _ => DiscoveringAction::Continue,
    }
}

fn handle_opening_event(event: Event) -> OpeningAction {
    match event {
        Event::Key(key)
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                && is_quit_key(key) =>
        {
            OpeningAction::Quit
        }
        _ => OpeningAction::Continue,
    }
}

fn is_quit_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        || key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
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
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    use super::{
        BASELINE_PARTICLE_COUNT, LandingApp, LandingData, LandingSuggestion, MAX_PARTICLE_COUNT,
        OpeningAction, handle_opening_event, particle_count, render,
    };
    use crate::diff::{DiffStats, DiffTarget};

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
    fn opening_phase_ignores_selection_input_but_accepts_quit() {
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            handle_opening_event(enter),
            OpeningAction::Continue
        ));

        let quit = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(handle_opening_event(quit), OpeningAction::Quit));
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
            }),
            BASELINE_PARTICLE_COUNT + 4
        );

        assert_eq!(
            particle_count(DiffStats {
                files: MAX_PARTICLE_COUNT.saturating_mul(2),
                additions: 0,
                deletions: 0,
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
