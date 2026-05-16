use once_cell::sync::Lazy;
use ratatui::{
    style::{Color, Style},
    text::Span,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};

static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(SyntaxSet::load_defaults_newlines);
static THEME: Lazy<Theme> = Lazy::new(|| {
    ThemeSet::load_defaults()
        .themes
        .get("base16-ocean.dark")
        .cloned()
        .expect("default theme exists")
});

#[derive(Clone, Copy)]
pub enum DiffKind {
    Context,
    Added,
    Removed,
}

pub struct FileHighlighter<'a> {
    inner: Option<HighlightLines<'a>>,
}

impl FileHighlighter<'static> {
    pub fn new(path: &str) -> Self {
        let inner = syntax_for_path(path).map(|syntax| HighlightLines::new(syntax, &THEME));
        Self { inner }
    }

    pub fn highlight_line(&mut self, text: &str, diff_kind: DiffKind) -> Vec<Span<'static>> {
        let background = match diff_kind {
            DiffKind::Context => None,
            DiffKind::Added => Some(Color::Rgb(18, 48, 24)),
            DiffKind::Removed => Some(Color::Rgb(60, 24, 24)),
        };

        match &mut self.inner {
            Some(highlighter) => highlighter
                .highlight_line(text, &SYNTAX_SET)
                .map(|ranges| {
                    ranges
                        .into_iter()
                        .map(|(style, segment)| {
                            Span::styled(segment.to_string(), merge_style(style, background))
                        })
                        .collect()
                })
                .unwrap_or_else(|_| vec![plain_span(text, background)]),
            None => vec![plain_span(text, background)],
        }
    }
}

fn syntax_for_path(path: &str) -> Option<&'static SyntaxReference> {
    if !is_supported_path(path) {
        return None;
    }

    SYNTAX_SET.find_syntax_for_file(path).ok().flatten()
}

fn is_supported_path(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .map(|ext| {
            matches!(
                ext,
                "rs" | "js" | "ts" | "sh" | "bash" | "md" | "markdown" | "html" | "css" | "json"
            )
        })
        .unwrap_or(false)
}

fn merge_style(style: SyntectStyle, background: Option<Color>) -> Style {
    let mut merged = Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));

    if let Some(background) = background {
        merged = merged.bg(background);
    }

    merged
}

fn plain_span(text: &str, background: Option<Color>) -> Span<'static> {
    let mut style = Style::default();
    if let Some(background) = background {
        style = style.bg(background);
    }

    Span::styled(text.to_string(), style)
}
