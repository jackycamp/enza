mod agent;
mod cli;
mod core;
mod git;
mod log;
mod tui;

use std::{io, path::PathBuf};

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let repo_path = cli.repo.clone().unwrap_or_else(|| PathBuf::from("."));
    let direct_diff = cli.command.as_ref().map(|Command::Diff(args)| {
        (
            args.diff_target().unwrap_or_else(|error| error.exit()),
            args.diff_filter().unwrap_or_else(|error| error.exit()),
        )
    });

    tui::run(&repo_path, direct_diff)
}
