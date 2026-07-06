use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand, error::ErrorKind};

use crate::diff::{DiffFilter, DiffTarget};

#[derive(Debug, Parser)]
#[command(name = "enza", version, about = "Terminal diff viewer")]
pub struct Cli {
    #[arg(long, value_name = "PATH", global = true)]
    pub repo: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Open a diff directly")]
    Diff(DiffArgs),
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    #[arg(long)]
    pub cached: bool,

    #[arg(long = "diff-filter", value_name = "FILTER")]
    diff_filter: Option<String>,

    #[arg(value_name = "REVISION")]
    revision: Option<String>,
}

impl DiffArgs {
    pub fn diff_target(&self) -> Result<DiffTarget, clap::Error> {
        if self.cached {
            if self.revision.is_some() {
                return Err(Cli::command().error(
                    ErrorKind::ArgumentConflict,
                    "`--cached` cannot be used with a revision range",
                ));
            }

            return Ok(DiffTarget::Cached);
        }

        let Some(revision) = &self.revision else {
            return Ok(DiffTarget::Worktree);
        };

        if let Some((base, head)) = revision.split_once("...") {
            return validate_range(base, head, revision, true);
        }

        if let Some((base, head)) = revision.split_once("..") {
            return validate_range(base, head, revision, false);
        }

        Err(Cli::command().error(
            ErrorKind::ValueValidation,
            format!(
                "unsupported revision `{revision}`; expected `<base>...<head>` or `<base>..<head>`"
            ),
        ))
    }

    pub fn diff_filter(&self) -> Result<Option<DiffFilter>, clap::Error> {
        let Some(value) = &self.diff_filter else {
            return Ok(None);
        };

        DiffFilter::parse(value).map(Some).ok_or_else(|| {
            Cli::command().error(
                ErrorKind::ValueValidation,
                format!(
                    "unsupported diff filter `{value}`; expected Git diff-filter letters like `M`, `AM`, or `ad`"
                ),
            )
        })
    }
}

fn validate_range(
    base: &str,
    head: &str,
    original: &str,
    merge_base: bool,
) -> Result<DiffTarget, clap::Error> {
    if base.is_empty() || head.is_empty() {
        return Err(Cli::command().error(
            ErrorKind::ValueValidation,
            format!("invalid revision range `{original}`"),
        ));
    }

    if merge_base {
        Ok(DiffTarget::MergeBaseRange {
            base: base.to_string(),
            head: head.to_string(),
        })
    } else {
        Ok(DiffTarget::Range {
            base: base.to_string(),
            head: head.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command};
    use crate::diff::DiffTarget;

    #[test]
    fn bare_command_opens_landing_mode() {
        let cli = Cli::try_parse_from(["enza"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn diff_command_defaults_to_worktree() {
        let cli = Cli::try_parse_from(["enza", "diff"]).unwrap();
        let Some(Command::Diff(args)) = cli.command else {
            panic!("expected diff command");
        };

        assert_eq!(args.diff_target().unwrap(), DiffTarget::Worktree);
    }

    #[test]
    fn diff_command_accepts_revision_ranges() {
        let cli = Cli::try_parse_from(["enza", "diff", "main...HEAD"]).unwrap();
        let Some(Command::Diff(args)) = cli.command else {
            panic!("expected diff command");
        };

        assert_eq!(
            args.diff_target().unwrap(),
            DiffTarget::MergeBaseRange {
                base: "main".to_string(),
                head: "HEAD".to_string(),
            }
        );
    }

    #[test]
    fn diff_command_rejects_cached_revision_range() {
        let cli = Cli::try_parse_from(["enza", "diff", "--cached", "main...HEAD"]).unwrap();
        let Some(Command::Diff(args)) = cli.command else {
            panic!("expected diff command");
        };

        assert!(args.diff_target().is_err());
    }
}
