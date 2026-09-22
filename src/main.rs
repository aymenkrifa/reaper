#[cfg(not(target_os = "linux"))]
compile_error!("reaper currently only supports Linux (it reads /proc directly)");

mod app;
mod cli;
mod lsof;
mod ui;

use std::process::ExitCode;

use clap::Parser;

/// Re-run the official installer, targeting the directory this binary
/// runs from so the update lands in place regardless of where reaper was
/// installed. The binary itself stays network-free: curl fetches, and the
/// installer keeps sole ownership of checksum verification and messaging.
fn self_update() -> color_eyre::Result<()> {
    let exe = std::env::current_exe()?;
    let Some(bin_dir) = exe.parent() else {
        return Err(color_eyre::eyre::eyre!(
            "could not determine where reaper is installed (running from {})",
            exe.display()
        ));
    };
    let status = std::process::Command::new("sh")
        .args([
            "-c",
            "curl -LsSf https://reaper.aymenkrifa.com/install.sh | sh",
        ])
        .env("REAPER_BIN_DIR", bin_dir)
        .status()?;
    if !status.success() {
        eprintln!(
            "\nupdate failed — if reaper lives in a system directory, try: sudo reaper update"
        );
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn main() -> color_eyre::Result<ExitCode> {
    // Everything except the bare TUI must work without a terminal: the
    // installer parses `reaper --version`, and `list`/`kill` are meant for
    // scripts. So parse and dispatch before any TUI setup.
    let cli = cli::Cli::parse();
    match cli.command {
        Some(cli::Command::List {
            ports,
            user,
            all,
            json,
        }) => return Ok(cli::list(&ports, user.as_deref(), all, json)),
        Some(cli::Command::Kill { ports, yes }) => return Ok(cli::kill(&ports, yes)),
        Some(cli::Command::Update) => {
            self_update()?;
            return Ok(ExitCode::SUCCESS);
        }
        None => {}
    }

    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = app::App::new()
        .with_search(cli.query.unwrap_or_default())
        .run(terminal);
    ratatui::restore();
    result.map(|()| ExitCode::SUCCESS)
}
