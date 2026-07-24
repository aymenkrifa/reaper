#[cfg(not(target_os = "linux"))]
compile_error!("reaper currently only supports Linux (it reads /proc directly)");

mod app;
mod lsof;
mod ui;

const HELP: &str = "\
reaper — a linux tui for listing and killing listening ports

Usage: reaper [OPTIONS | COMMAND]

Commands:
  update         Download and install the latest release

Options:
  -h, --help     Print this help
  -V, --version  Print the version

Keys (inside the TUI):
  ↑/↓ navigate • ⏎ kill (with confirmation) • / search
  s or 1-7 sort • a show restricted • r refresh • q/Esc quit

Run with sudo to see and kill other users' listeners.
Docs: https://reaper.aymenkrifa.com";

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

fn main() -> color_eyre::Result<()> {
    // The installer parses `reaper --version` to report updates, so this
    // must work without a terminal and before any TUI setup.
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("reaper {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!("{}", HELP);
                return Ok(());
            }
            "update" => return self_update(),
            other => {
                eprintln!("unknown option: {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }

    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = app::App::new().run(terminal);
    ratatui::restore();
    result
}
