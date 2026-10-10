#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use canopy::ui;

fn main() -> eframe::Result {
    #[cfg(not(windows))]
    {
        eprintln!("Diskvale requires Windows and an NTFS drive.");
        std::process::exit(1);
    }
    #[cfg(windows)]
    if unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() } == 0 {
        eprintln!("Diskvale requires administrator access. Start it with Run as administrator.");
        std::process::exit(1);
    }
    if std::env::args().any(|a| a == "--scan") {
        ui::cli();
        return Ok(());
    }
    let compact = std::env::args().any(|arg| arg == "--compact");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(if compact {
                [1050.0, 700.0]
            } else {
                [1400.0, 900.0]
            })
            .with_min_inner_size([1050.0, 700.0])
            .with_title(ui::brand::WINDOW_TITLE)
            .with_icon(ui::brand::window_icon()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        ui::brand::NAME,
        options,
        Box::new(|cc| Ok(Box::new(ui::Canopy::new(cc)))),
    )
}
