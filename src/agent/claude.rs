use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::agent::ProviderOutput;
use crate::note::{AgentFailure, AgentFailureKind};

pub(super) fn command(repo_root: &Path, session_id: Option<&str>) -> Command {
    let mut command = Command::new("claude");
    command.current_dir(repo_root).args([
        "--print",
        "--output-format",
        "json",
        "--permission-mode",
        "dontAsk",
        "--tools",
        "Read,Glob,Grep",
    ]);
    if let Some(session_id) = session_id {
        command.args(["--resume", session_id]);
    }
    command
}

pub(super) fn parse_output(
    stdout: &str,
    existing_session_id: Option<&str>,
) -> Result<ProviderOutput, AgentFailure> {
    let result: Value = serde_json::from_str(stdout.trim()).map_err(|error| {
        AgentFailure::new(
            AgentFailureKind::InvalidResponse,
            format!("Claude returned invalid JSON: {error}"),
            true,
        )
    })?;

    let response = result.get("result").and_then(Value::as_str);
    if result.get("is_error").and_then(Value::as_bool) == Some(true) {
        return Err(AgentFailure::new(
            AgentFailureKind::ProcessExit,
            response.unwrap_or("Claude reported an error."),
            true,
        ));
    }

    let response = response.ok_or_else(|| {
        AgentFailure::new(
            AgentFailureKind::MissingResponse,
            "Claude completed without returning a message.",
            true,
        )
    })?;
    let session_id = result
        .get("session_id")
        .and_then(Value::as_str)
        .or(existing_session_id)
        .ok_or_else(|| {
            AgentFailure::new(
                AgentFailureKind::InvalidResponse,
                "Claude did not return a session ID.",
                true,
            )
        })?;

    Ok(ProviderOutput {
        session_id: session_id.to_string(),
        response: response.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_command_keeps_read_only_tools_and_repository() {
        let command = command(Path::new("/repo"), Some("session-1"));
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(command.get_current_dir(), Some(Path::new("/repo")));
        assert_eq!(
            args,
            [
                "--print",
                "--output-format",
                "json",
                "--permission-mode",
                "dontAsk",
                "--tools",
                "Read,Glob,Grep",
                "--resume",
                "session-1",
            ]
        );
    }

    #[test]
    fn parses_session_and_result() {
        let output = parse_output(
            r#"{"type":"result","subtype":"success","is_error":false,"session_id":"session-1","result":"A useful reply"}"#,
            None,
        )
        .unwrap();

        assert_eq!(output.session_id, "session-1");
        assert_eq!(output.response, "A useful reply");
    }

    #[test]
    fn surfaces_provider_errors() {
        let error = parse_output(
            r#"{"type":"result","is_error":true,"session_id":"session-1","result":"Authentication required"}"#,
            None,
        )
        .unwrap_err();
        assert!(error.message.contains("Authentication"));
    }
}
