use std::path::PathBuf;

use clap::{CommandFactory, Parser, error::ErrorKind};

use crate::diff::{DiffFilter, DiffTarget};

#[derive(Debug, Parser)]
#[command(name = "enza", version, about = "Terminal diff viewer")]
pub struct Cli {
    #[arg(long)]
    pub cached: bool,

    #[arg(long = "diff-filter", value_name = "FILTER")]
    diff_filter: Option<String>,

    #[arg(long, value_name = "PATH")]
    pub repo: Option<PathBuf>,

    #[arg(value_name = "REVISION")]
    revision: Option<String>,
}

impl Cli {
    pub fn diff_target(&self) -> Result<DiffTarget, clap::Error> {
        if self.cached {
            if self.revision.is_some() {
                return Err(Self::command().error(
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

        Err(Self::command().error(
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
            Self::command().error(
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
