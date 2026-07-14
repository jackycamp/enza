use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

use crate::note::AgentProvider;

pub const CODEX_COLOR: Color = Color::Rgb(180, 120, 255);
pub const CLAUDE_COLOR: Color = Color::Rgb(255, 159, 67);
pub const ACTIVITY_PLUS_COLOR: Color = Color::Rgb(80, 200, 120);
pub const ACTIVITY_MINUS_COLOR: Color = Color::Rgb(255, 95, 95);

pub fn styled_mentions(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let lowercase = text.to_ascii_lowercase();
    let providers = [AgentProvider::Codex, AgentProvider::Claude];
    let mut spans = Vec::new();
    let mut emitted_through = 0;
    let mut search_from = 0;

    while search_from < text.len() {
        let next = providers
            .iter()
            .filter_map(|provider| {
                lowercase[search_from..]
                    .find(provider.mention())
                    .map(|offset| (search_from + offset, *provider))
            })
            .min_by_key(|(index, _)| *index);
        let Some((start, provider)) = next else {
            break;
        };
        let end = start + provider.mention().len();

        if !has_mention_boundaries(text, start, end) {
            search_from = start + 1;
            continue;
        }

        if emitted_through < start {
            spans.push(Span::styled(
                text[emitted_through..start].to_string(),
                base_style,
            ));
        }
        spans.push(Span::styled(
            text[start..end].to_string(),
            base_style.fg(provider_color(provider)),
        ));
        emitted_through = end;
        search_from = end;
    }

    if emitted_through < text.len() {
        spans.push(Span::styled(
            text[emitted_through..].to_string(),
            base_style,
        ));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base_style));
    }
    spans
}

pub fn styled_note_text(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let leading_spaces = text.len() - text.trim_start_matches(' ').len();
    let (leading, content) = text.split_at(leading_spaces);
    if let Some((glyph, color)) = [("+", ACTIVITY_PLUS_COLOR), ("−", ACTIVITY_MINUS_COLOR)]
        .into_iter()
        .find(|(glyph, _)| content.starts_with(glyph))
    {
        let mut spans = Vec::new();
        if !leading.is_empty() {
            spans.push(Span::styled(leading.to_string(), base_style));
        }
        spans.push(Span::styled(
            glyph.to_string(),
            base_style.fg(color).add_modifier(Modifier::BOLD),
        ));
        spans.extend(styled_mentions(
            &content[glyph.len()..],
            base_style.fg(Color::DarkGray),
        ));
        return spans;
    }
    if is_subtle_note_metadata(content) {
        let mut spans = Vec::new();
        if !leading.is_empty() {
            spans.push(Span::styled(leading.to_string(), base_style));
        }
        spans.extend(styled_mentions(content, base_style.fg(Color::DarkGray)));
        return spans;
    }
    let author = [
        ("You:", None),
        ("Codex:", Some(AgentProvider::Codex)),
        ("Claude:", Some(AgentProvider::Claude)),
    ]
    .into_iter()
    .find(|(label, _)| content.starts_with(label));

    let Some((label, provider)) = author else {
        return styled_mentions(text, base_style);
    };

    let mut spans = Vec::new();
    if !leading.is_empty() {
        spans.push(Span::styled(leading.to_string(), base_style));
    }
    let author_style = provider
        .map(|provider| base_style.fg(provider_color(provider)))
        .unwrap_or(base_style)
        .add_modifier(Modifier::BOLD);
    spans.push(Span::styled(label.to_string(), author_style));
    spans.extend(styled_mentions(&content[label.len()..], base_style));
    spans
}

fn is_subtle_note_metadata(content: &str) -> bool {
    content.starts_with("Enter to ")
        || content.starts_with("Waiting to start")
        || content.contains(" earlier messages")
        || content.contains(" more messages")
}

pub fn provider_color(provider: AgentProvider) -> Color {
    match provider {
        AgentProvider::Codex => CODEX_COLOR,
        AgentProvider::Claude => CLAUDE_COLOR,
    }
}

fn has_mention_boundaries(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    before.is_none_or(is_mention_boundary) && after.is_none_or(is_mention_boundary)
}

fn is_mention_boundary(character: char) -> bool {
    !character.is_alphanumeric() && character != '_' && character != '@'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_mentions_receive_provider_colors() {
        let spans = styled_mentions("Ask @CODEX, then @claude.", Style::default());

        assert_eq!(
            spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "Ask @CODEX, then @claude."
        );
        assert_eq!(
            spans
                .iter()
                .find(|span| span.content == "@CODEX")
                .unwrap()
                .style
                .fg,
            Some(CODEX_COLOR)
        );
        assert_eq!(
            spans
                .iter()
                .find(|span| span.content == "@claude")
                .unwrap()
                .style
                .fg,
            Some(CLAUDE_COLOR)
        );
    }

    #[test]
    fn partial_and_escaped_mentions_are_not_colored() {
        let spans = styled_mentions("@codexchange @@claude", Style::default());

        assert!(spans.iter().all(|span| span.style.fg.is_none()));
    }

    #[test]
    fn thread_authors_are_bold_and_agents_use_the_provider_color() {
        let codex = styled_note_text(" Codex: response", Style::default());
        let codex_author = codex.iter().find(|span| span.content == "Codex:").unwrap();
        let claude = styled_note_text(" Claude: response", Style::default());
        let claude_author = claude
            .iter()
            .find(|span| span.content == "Claude:")
            .unwrap();
        let user = styled_note_text(" You: question", Style::default());
        let user_author = user.iter().find(|span| span.content == "You:").unwrap();

        assert_eq!(codex_author.style.fg, Some(CODEX_COLOR));
        assert_eq!(claude_author.style.fg, Some(CLAUDE_COLOR));
        assert!(codex_author.style.add_modifier.contains(Modifier::BOLD));
        assert!(claude_author.style.add_modifier.contains(Modifier::BOLD));
        assert!(user_author.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn activity_and_action_rows_are_subtle_with_a_colored_pulse() {
        let plus = styled_note_text(" + Responding…", Style::default().fg(Color::White));
        let minus = styled_note_text(" − Responding…", Style::default().fg(Color::White));
        let hint = styled_note_text(
            " Enter to collapse · n to reply",
            Style::default().fg(Color::White),
        );

        assert_eq!(
            plus.iter()
                .find(|span| span.content == "+")
                .unwrap()
                .style
                .fg,
            Some(ACTIVITY_PLUS_COLOR)
        );
        assert_eq!(
            minus
                .iter()
                .find(|span| span.content == "−")
                .unwrap()
                .style
                .fg,
            Some(ACTIVITY_MINUS_COLOR)
        );
        assert!(plus.iter().any(|span| {
            span.content.contains("Responding") && span.style.fg == Some(Color::DarkGray)
        }));
        assert!(
            hint.iter()
                .filter(|span| !span.content.trim().is_empty())
                .all(|span| span.style.fg == Some(Color::DarkGray))
        );
    }
}
