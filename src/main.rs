use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "gistui",
    version,
    about = "Manage local config files and GitHub gist files"
)]
struct Cli {
    #[arg(long, help = "Print startup checks without launching the TUI")]
    check: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.check {
        gistui::gh::check_gh_ready()?;
        println!("gh is installed and authenticated");
        return Ok(());
    }

    gistui::tui::run()
}
