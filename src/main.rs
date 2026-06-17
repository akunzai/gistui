use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "gistui",
    version,
    about = "A terminal UI for managing GitHub Gists"
)]
struct Cli {
    #[arg(long, help = "Print startup checks without launching the TUI")]
    check: bool,
    #[arg(
        long,
        help = "Upgrade the running pre-built binary from GitHub Releases (combine with --check or --upgrade-version)"
    )]
    upgrade: bool,
    #[arg(
        long = "upgrade-version",
        value_name = "TAG",
        help = "With --upgrade: install a specific release (v0.12.0 or 0.12.0)"
    )]
    upgrade_version: Option<String>,
    #[arg(
        value_name = "PATH",
        help = "Working directory to pair files against (defaults to the current directory)"
    )]
    path: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.upgrade {
        return gistui::upgrade::run(gistui::upgrade::Options {
            check_only: cli.check,
            version: cli.upgrade_version,
        });
    }

    // Validate the working directory before touching the terminal, so a bad path fails
    // cleanly with a non-zero exit instead of half-entering the TUI.
    let workdir = gistui::config::resolve_working_dir(cli.path)?;
    std::env::set_current_dir(&workdir)
        .map_err(|e| anyhow::anyhow!("could not enter {}: {e}", workdir.display()))?;

    if cli.check {
        gistui::gh::check_gh_ready()?;
        println!("gh is installed and authenticated");
        return Ok(());
    }

    gistui::tui::run()
}
