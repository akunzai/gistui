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
        value_name = "PATH",
        help = "Working directory to pair files against (defaults to the current directory)"
    )]
    path: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

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
