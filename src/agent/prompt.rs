//! Review prompt construction for new agent conversations.
//!
//! Prompts identify the comparison and selected note target, then include the
//! relevant diff hunks. Excerpts are capped at 160 lines and 24 KiB.

use std::path::PathBuf;

use crate::diff::{DiffFile, DiffLine, DiffSession, DiffTarget};
use crate::note::NoteTarget;

const MAX_CONTEXT_LINES: usize = 160;
const MAX_CONTEXT_BYTES: usize = 24 * 1024;

#[derive(Clone, Debug)]
pub struct ReviewContext {
    pub repo_root: PathBuf,
    pub diff_target: DiffTarget,
}

impl ReviewContext {
    pub fn new(repo_root: PathBuf, diff_target: DiffTarget) -> Self {
        Self {
            repo_root,
            diff_target,
        }
    }
}

pub fn build_agent_prompt(
    review: &ReviewContext,
    session: &DiffSession,
    target: &NoteTarget,
    question: &str,
) -> String {
    let target_description = describe_target(target);
    let diff_description = describe_diff_target(&review.diff_target);
    let excerpt = build_diff_excerpt(session, target);

    format!(
        "You are replying to a code review note in Enza.\n\n\
         Answer the user's question directly.\n\
         Do not modify files.\n\
         Return concise plain text suitable for a terminal.\n\
         Do not include Markdown tables.\n\n\
         Diff target: {diff_description}\n\
         Location: {target_description}\n\n\
         Selected diff:\n\
         {excerpt}\n\n\
         User message:\n\
         {question}"
    )
}

fn describe_diff_target(target: &DiffTarget) -> String {
    match target {
        DiffTarget::Worktree => "working tree".to_string(),
        DiffTarget::Cached => "staged changes".to_string(),
        DiffTarget::Range { base, head } => format!("revision range {base}..{head}"),
        DiffTarget::MergeBaseRange { base, head } => {
            format!("merge-base revision range {base}...{head}")
        }
    }
}

fn describe_target(target: &NoteTarget) -> String {
    match target {
        NoteTarget::File { file_path } => file_path.clone(),
        NoteTarget::Hunk {
            file_path,
            hunk_header,
        } => format!("{file_path}, hunk {hunk_header}"),
        NoteTarget::Line {
            file_path,
            old_lineno,
            new_lineno,
        } => format!(
            "{file_path}, old line {}, new line {}",
            display_lineno(*old_lineno),
            display_lineno(*new_lineno)
        ),
        NoteTarget::Range {
            file_path,
            start_old_lineno,
            start_new_lineno,
            end_old_lineno,
            end_new_lineno,
        } => format!(
            "{file_path}, old lines {} to {}, new lines {} to {}",
            display_lineno(*start_old_lineno),
            display_lineno(*end_old_lineno),
            display_lineno(*start_new_lineno),
            display_lineno(*end_new_lineno)
        ),
    }
}

fn display_lineno(lineno: Option<usize>) -> String {
    lineno
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn build_diff_excerpt(session: &DiffSession, target: &NoteTarget) -> String {
    let file_path = match target {
        NoteTarget::File { file_path }
        | NoteTarget::Hunk { file_path, .. }
        | NoteTarget::Line { file_path, .. }
        | NoteTarget::Range { file_path, .. } => file_path,
    };
    let Some(file) = session.files.iter().find(|file| &file.path == file_path) else {
        return "(The selected diff is no longer available.)".to_string();
    };

    let selected_hunks = selected_hunk_indexes(file, target);
    let mut excerpt = String::new();
    let mut line_count = 0usize;
    let mut truncated = false;
    append_context_line(&mut excerpt, &format!("--- {}", file.old_path));
    append_context_line(&mut excerpt, &format!("+++ {}", file.new_path));

    for hunk_index in selected_hunks {
        let Some(hunk) = file.hunks.get(hunk_index) else {
            continue;
        };
        if !try_append_context_line(&mut excerpt, &hunk.header, &mut line_count) {
            truncated = true;
            break;
        }
        for line in &hunk.lines {
            let rendered = match line {
                DiffLine::Context { text, .. } => format!(" {text}"),
                DiffLine::Added { text, .. } => format!("+{text}"),
                DiffLine::Removed { text, .. } => format!("-{text}"),
            };
            if !try_append_context_line(&mut excerpt, &rendered, &mut line_count) {
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
    }

    if truncated {
        append_context_line(&mut excerpt, "… diff context truncated …");
    }
    excerpt.trim_end().to_string()
}

fn selected_hunk_indexes(file: &DiffFile, target: &NoteTarget) -> Vec<usize> {
    match target {
        NoteTarget::File { .. } => (0..file.hunks.len()).collect(),
        NoteTarget::Hunk { hunk_header, .. } => file
            .hunks
            .iter()
            .position(|hunk| &hunk.header == hunk_header)
            .into_iter()
            .collect(),
        NoteTarget::Line {
            old_lineno,
            new_lineno,
            ..
        } => file
            .hunks
            .iter()
            .position(|hunk| {
                hunk.lines
                    .iter()
                    .any(|line| line_matches(line, *old_lineno, *new_lineno))
            })
            .into_iter()
            .collect(),
        NoteTarget::Range {
            start_old_lineno,
            start_new_lineno,
            end_old_lineno,
            end_new_lineno,
            ..
        } => {
            let start = file.hunks.iter().position(|hunk| {
                hunk.lines
                    .iter()
                    .any(|line| line_matches(line, *start_old_lineno, *start_new_lineno))
            });
            let end = file.hunks.iter().rposition(|hunk| {
                hunk.lines
                    .iter()
                    .any(|line| line_matches(line, *end_old_lineno, *end_new_lineno))
            });
            match (start, end) {
                (Some(start), Some(end)) if start <= end => (start..=end).collect(),
                _ => (0..file.hunks.len()).collect(),
            }
        }
    }
}

fn line_matches(line: &DiffLine, old_lineno: Option<usize>, new_lineno: Option<usize>) -> bool {
    match line {
        DiffLine::Context {
            old_lineno: old,
            new_lineno: new,
            ..
        } => old_lineno == Some(*old) && new_lineno == Some(*new),
        DiffLine::Added {
            new_lineno: new, ..
        } => old_lineno.is_none() && new_lineno == Some(*new),
        DiffLine::Removed {
            old_lineno: old, ..
        } => old_lineno == Some(*old) && new_lineno.is_none(),
    }
}

fn try_append_context_line(output: &mut String, line: &str, line_count: &mut usize) -> bool {
    if *line_count >= MAX_CONTEXT_LINES
        || output.len().saturating_add(line.len()).saturating_add(1) > MAX_CONTEXT_BYTES
    {
        return false;
    }
    append_context_line(output, line);
    *line_count += 1;
    true
}

fn append_context_line(output: &mut String, line: &str) {
    output.push_str(line);
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffHunk, DiffLine};

    #[test]
    fn line_prompts_include_the_containing_hunk_and_review_instruction() {
        let prompt = build_agent_prompt(
            &ReviewContext::new(PathBuf::from("/repo"), DiffTarget::Worktree),
            &session(),
            &NoteTarget::Line {
                file_path: "src/lib.rs".to_string(),
                old_lineno: None,
                new_lineno: Some(2),
            },
            "Why add this?",
        );

        assert!(prompt.contains("Do not modify files."));
        assert!(prompt.contains("@@ -1,1 +1,2 @@"));
        assert!(prompt.contains("+new line"));
        assert!(prompt.contains("Why add this?"));
    }

    #[test]
    fn missing_targets_produce_explicit_context() {
        let excerpt = build_diff_excerpt(
            &session(),
            &NoteTarget::File {
                file_path: "missing.rs".to_string(),
            },
        );
        assert_eq!(excerpt, "(The selected diff is no longer available.)");
    }

    fn session() -> DiffSession {
        DiffSession {
            files: vec![DiffFile {
                path: "src/lib.rs".to_string(),
                old_path: "src/lib.rs".to_string(),
                new_path: "src/lib.rs".to_string(),
                hunks: vec![DiffHunk {
                    header: "@@ -1,1 +1,2 @@".to_string(),
                    lines: vec![
                        DiffLine::Context {
                            old_lineno: 1,
                            new_lineno: 1,
                            text: "same".to_string(),
                        },
                        DiffLine::Added {
                            new_lineno: 2,
                            text: "new line".to_string(),
                        },
                    ],
                }],
            }],
        }
    }
}
