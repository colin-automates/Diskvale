//! Developer preview of the production UI from an existing saved scan.
//! Usage: cargo run --example map_review -- --snapshot scan.json [--compact]
//! The published application's administrator launch is unchanged.
use canopy::ui;

struct Review {
    app: ui::Canopy,
    capture: Option<std::path::PathBuf>,
    started: std::time::Instant,
    requested: bool,
    query: Option<String>,
    query_focused: bool,
}

impl eframe::App for Review {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        if self.query.is_some() && self.started.elapsed().as_secs() >= 2 {
            if self.query_focused {
                let query = self.query.take().unwrap();
                ctx.input_mut(|input| input.events.push(eframe::egui::Event::Text(query)));
            } else {
                ctx.input_mut(|input| {
                    input.events.push(eframe::egui::Event::Key {
                        key: eframe::egui::Key::K,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: eframe::egui::Modifiers::CTRL,
                    })
                });
                self.query_focused = true;
            }
        }
        self.app.update(ctx, frame);
        let Some(path) = &self.capture else { return };
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let eframe::egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let mut ppm = format!("P6\n{} {}\n255\n", image.size[0], image.size[1]).into_bytes();
            for pixel in &image.pixels {
                ppm.extend_from_slice(&pixel.to_array()[..3]);
            }
            std::fs::write(path, ppm).expect("write rendered review frame");
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        } else if !self.requested && self.started.elapsed().as_secs() >= 12 {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if !args.windows(2).any(|pair| pair[0] == "--snapshot")
        || args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--scan" | "--auto-scan" | "--mft-file"))
    {
        eprintln!("This UI review requires --snapshot and cannot start a live scan.");
        std::process::exit(1);
    }
    // A standard token cannot open the bulk scanner. This helper is solely for
    // reviewing saved data; elevated users should use the published executable.
    #[cfg(windows)]
    if unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() } != 0 {
        eprintln!("Use a standard terminal for the saved-scan UI review.");
        std::process::exit(1);
    }
    let compact = args.iter().any(|arg| arg == "--compact");
    let query = args
        .iter()
        .position(|arg| arg == "--query")
        .and_then(|index| args.get(index + 1))
        .cloned();
    let capture = args
        .iter()
        .position(|arg| arg == "--capture")
        .and_then(|index| args.get(index + 1))
        .map(std::path::PathBuf::from);
    eframe::run_native(
        "Diskvale — Saved scan review",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size(if compact {
                    [1050.0, 700.0]
                } else {
                    [1400.0, 900.0]
                })
                .with_min_inner_size([1050.0, 700.0])
                .with_icon(ui::brand::window_icon()),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Review {
                app: ui::Canopy::new(cc),
                capture,
                started: std::time::Instant::now(),
                requested: false,
                query,
                query_focused: false,
            }))
        }),
    )
}
