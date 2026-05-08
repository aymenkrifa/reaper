#[cfg(not(target_os = "linux"))]
compile_error!("reaper currently only supports Linux (it reads /proc directly)");

mod app;
mod lsof;
mod ui;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = app::App::new().run(terminal);
    ratatui::restore();
    result
}
