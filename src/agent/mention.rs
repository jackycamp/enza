//! Provider mention parsing for note input.
//!
//! A leading `@codex` or `@claude` mention creates an agent prompt. Other text
//! creates a personal note, and a doubled `@` escapes a recognised mention.

use crate::note::AgentProvider;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParsedNoteInput {
    Personal(String),
    Agent {
        provider: AgentProvider,
        prompt: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MentionParseError {
    EmptyPrompt(AgentProvider),
}

pub fn parse_note_input(input: &str) -> Result<ParsedNoteInput, MentionParseError> {
    let input = input.trim();

    for provider in [AgentProvider::Codex, AgentProvider::Claude] {
        let mention = provider.mention();
        let escaped = format!("@{mention}");
        if starts_with_ignore_ascii_case(input, &escaped)
            && has_mention_boundary(&input[escaped.len()..])
        {
            return Ok(ParsedNoteInput::Personal(input[1..].to_string()));
        }

        if !starts_with_ignore_ascii_case(input, mention) {
            continue;
        }

        let rest = &input[mention.len()..];
        if !has_mention_boundary(rest) {
            continue;
        }

        let prompt = rest
            .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ':'))
            .trim();
        if prompt.is_empty() {
            return Err(MentionParseError::EmptyPrompt(provider));
        }

        return Ok(ParsedNoteInput::Agent {
            provider,
            prompt: prompt.to_string(),
        });
    }

    Ok(ParsedNoteInput::Personal(input.to_string()))
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

fn has_mention_boundary(rest: &str) -> bool {
    rest.is_empty()
        || rest
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace() || matches!(ch, ',' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_known_mentions_create_agent_inputs() {
        assert_eq!(
            parse_note_input("@codex, explain this").unwrap(),
            ParsedNoteInput::Agent {
                provider: AgentProvider::Codex,
                prompt: "explain this".to_string(),
            }
        );
        assert_eq!(
            parse_note_input("@CLAUDE: check this").unwrap(),
            ParsedNoteInput::Agent {
                provider: AgentProvider::Claude,
                prompt: "check this".to_string(),
            }
        );
    }

    #[test]
    fn nonleading_and_partial_mentions_remain_personal_notes() {
        for body in ["ask @codex later", "@codexchange this", "@someone hello"] {
            assert_eq!(
                parse_note_input(body).unwrap(),
                ParsedNoteInput::Personal(body.to_string())
            );
        }
    }

    #[test]
    fn doubled_at_sign_escapes_a_known_mention() {
        assert_eq!(
            parse_note_input("@@codex remember this").unwrap(),
            ParsedNoteInput::Personal("@codex remember this".to_string())
        );
    }

    #[test]
    fn agent_mentions_require_a_prompt() {
        assert_eq!(
            parse_note_input("@claude:").unwrap_err(),
            MentionParseError::EmptyPrompt(AgentProvider::Claude)
        );
    }
}
