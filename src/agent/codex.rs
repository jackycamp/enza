//! Codex CLI command construction and response parsing.
//!
//! Commands run in the repository's read-only sandbox. Responses use Codex's
//! JSON event stream and retain its thread ID for follow-up requests.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::agent::ProviderOutput;
use crate::note::{AgentFailure, AgentFailureKind};

pub(super) fn command(repo_root: &Path, session_id: Option<&str>) -> Command {
    let mut command = Command::new("codex");
    command.args([
        "--sandbox",
        "read-only",
        "--ask-for-approval",
        "never",
        "--cd",
    ]);
    command.arg(repo_root);
    command.arg("exec");
    if let Some(session_id) = session_id {
        command.args(["resume", "--json", session_id, "-"]);
    } else {
        command.args(["--json", "-"]);
    }
    command
}

pub(super) fn parse_output(
    stdout: &str,
    existing_session_id: Option<&str>,
) -> Result<ProviderOutput, AgentFailure> {
    let mut session_id = existing_session_id.map(str::to_string);
    let mut response = None;
    let mut completed = false;

    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).map_err(|error| {
            AgentFailure::new(
                AgentFailureKind::InvalidResponse,
                format!("Codex returned invalid JSON: {error}"),
                true,
            )
        })?;
        match event.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                if let Some(value) = event.get("thread_id").and_then(Value::as_str) {
                    session_id = Some(value.to_string());
                }
            }
            Some("item.completed") => {
                let Some(item) = event.get("item") else {
                    continue;
                };
                if item.get("type").and_then(Value::as_str) == Some("agent_message")
                    && let Some(text) = item.get("text").and_then(Value::as_str)
                {
                    response = Some(text.to_string());
                }
            }
            Some("turn.completed") => completed = true,
            Some("turn.failed") | Some("error") => {
                return Err(AgentFailure::new(
                    AgentFailureKind::ProcessExit,
                    event_message(&event).unwrap_or_else(|| "Codex reported an error.".to_string()),
                    true,
                ));
            }
            _ => {}
        }
    }

    if !completed {
        return Err(AgentFailure::new(
            AgentFailureKind::MissingResponse,
            "Codex stopped before completing its response.",
            true,
        ));
    }
    let response = response.ok_or_else(|| {
        AgentFailure::new(
            AgentFailureKind::MissingResponse,
            "Codex completed without returning a message.",
            true,
        )
    })?;
    let session_id = session_id.ok_or_else(|| {
        AgentFailure::new(
            AgentFailureKind::InvalidResponse,
            "Codex did not return a session ID.",
            true,
        )
    })?;

    Ok(ProviderOutput {
        session_id,
        response,
    })
}

fn event_message(event: &Value) -> Option<String> {
    event
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| {
            event
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_command_keeps_read_only_policy_and_session_id() {
        let command = command(Path::new("/repo"), Some("thread-1"));
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            [
                "--sandbox",
                "read-only",
                "--ask-for-approval",
                "never",
                "--cd",
                "/repo",
                "exec",
                "resume",
                "--json",
                "thread-1",
                "-",
            ]
        );
    }

    #[test]
    fn parses_session_and_final_message_from_jsonl() {
        let output = parse_output(
            concat!(
                "{\"type\":\"thread.started\",\"thread_id\":\"thread-1\"}\n",
                "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"first\"}}\n",
                "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"final reply\"}}\n",
                "{\"type\":\"turn.completed\"}\n"
            ),
            None,
        )
        .unwrap();

        assert_eq!(output.session_id, "thread-1");
        assert_eq!(output.response, "final reply");
    }

    #[test]
    fn resumed_runs_can_reuse_the_existing_session_id() {
        let output = parse_output(
            concat!(
                "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"reply\"}}\n",
                "{\"type\":\"turn.completed\"}\n"
            ),
            Some("existing"),
        )
        .unwrap();
        assert_eq!(output.session_id, "existing");
    }
}
