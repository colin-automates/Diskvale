#![cfg(not(test))]
//! Capture only the production UI framebuffer, using a synthetic drive index.
//! No desktop pixels, file dialogs, live scan or filesystem actions are recorded.
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, pos2, vec2};
use std::{
    collections::{BTreeSet, HashMap},
    io::Write,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
};

#[allow(dead_code)]
mod ui {
    include!("../src/ui.rs");

    pub fn restrict_to_demo(app: &mut Canopy) {
        #[cfg(windows)]
        app.drives.clear();
    }
    pub fn ready(app: &Canopy) -> bool {
        app.scan.is_some() && app.receiver.is_none()
    }
    pub fn point(app: &Canopy, id: usize) -> egui::Pos2 {
        let tile = app
            .map_cache
            .iter()
            .find(|tile| tile.id == id)
            .expect("visible demo target");
        let rect = egui::Rect::from_min_size(
            egui::pos2(tile.rect.x, tile.rect.y),
            egui::vec2(tile.rect.w, tile.rect.h),
        );
        if tile.kind == treemap::Kind::Folder && tile.header {
            egui::pos2(rect.center().x, rect.top() + 8.0)
        } else {
            rect.center()
        }
    }
    pub fn state(app: &Canopy) -> serde_json::Value {
        serde_json::json!({"view": match app.view {View::Folders=>"Map",View::Largest=>"Files",View::Types=>"Types"},
            "scope":app.current,"query":app.query,"category":app.category.map(|v|v.label()),
            "selected":app.selected,"rows":app.rows.len(),"types":app.types_cache.len()})
    }
    pub fn assert_no_io(app: &Canopy) {
        assert!(app.receiver.is_none() && app.deletion.is_none() && app.delete_pending.is_none());
        assert!(
            app.scan_error.is_none(),
            "demo UI error: {:?}",
            app.scan_error
        );
        let scan = app.scan.as_ref().unwrap();
        assert_eq!(scan.root.to_str(), Some("Z:\\"));
        assert!(
            scan.stats
                .notices
                .iter()
                .any(|notice| notice == "DISKVALE_SYNTHETIC_DEMO_ONLY")
        );
    }
}

struct Capture {
    app: ui::Canopy,
    out: PathBuf,
    targets: HashMap<String, usize>,
    fps: usize,
    total: usize,
    index: usize,
    pending: bool,
    encoder: Option<Child>,
    pipe: Option<ChildStdin>,
    ffmpeg: PathBuf,
    position: Pos2,
    movement: Option<(usize, Pos2, Pos2)>,
    click_at: Option<usize>,
    labels: Vec<(String, Rect)>,
    texts: BTreeSet<String>,
    milestones: Vec<serde_json::Value>,
    started: bool,
}

fn painted_text(ctx: &egui::Context) -> Vec<(String, Rect)> {
    fn collect(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(t) => {
                result.push((t.galley.job.text.clone(), t.visual_bounding_rect()))
            }
            egui::Shape::Vec(v) => {
                for s in v {
                    collect(s, result)
                }
            }
            _ => {}
        }
    }
    ctx.graphics(|graphics| {
        let shapes = graphics.clone().drain(&[], &Default::default());
        let mut result = vec![];
        for clipped in shapes {
            collect(&clipped.shape, &mut result);
        }
        result
    })
}

impl Capture {
    fn at(&self, seconds: f64) -> bool {
        self.index == (seconds * self.fps as f64).round() as usize
    }
    fn label(&self, label: &str) -> Pos2 {
        self.labels
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("Missing demo control: {label}"))
            .1
            .center()
    }
    fn move_to(&mut self, point: Pos2) {
        self.movement = Some((self.index, self.position, point));
    }
    fn key(input: &mut egui::RawInput, key: egui::Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
    }
    fn script(&mut self, input: &mut egui::RawInput) {
        // All interactions are normal pointer/keyboard events delivered to the real UI.
        if self.at(2.0) {
            self.move_to(ui::point(&self.app, self.targets["showreel"]));
        }
        if self.at(6.5) {
            self.move_to(ui::point(&self.app, self.targets["media"]));
        }
        if self.at(7.4) {
            self.click_at = Some(self.index);
        }
        if self.at(7.8) {
            Self::key(input, egui::Key::Enter, egui::Modifiers::NONE);
        }
        if self.at(8.5) {
            self.move_to(ui::point(&self.app, self.targets["showreel"]));
        }
        if self.at(13.0) {
            self.move_to(self.label("Up"));
        }
        if self.at(13.8) {
            self.click_at = Some(self.index);
        }
        if self.at(14.4) {
            self.move_to(self.label("Files"));
        }
        if self.at(15.2) {
            self.click_at = Some(self.index);
        }
        if self.at(16.0) {
            self.move_to(pos2(660.0, 207.0));
        }
        if self.at(20.0) {
            self.move_to(pos2(500.0, 26.0));
            Self::key(input, egui::Key::K, egui::Modifiers::CTRL);
        }
        let start = (20.7 * self.fps as f64).round() as usize;
        let query = "ext:mp4 >1GB";
        for (i, c) in query.chars().enumerate() {
            if self.index == start + (i as f64 * 0.12 * self.fps as f64).round() as usize {
                input.events.push(egui::Event::Text(c.to_string()));
            }
        }
        if self.at(23.0) {
            self.move_to(pos2(840.0, 254.0));
        }
        if self.at(25.7) {
            self.move_to(self.label("×"));
        }
        if self.at(26.5) {
            self.click_at = Some(self.index);
        }
        if self.at(27.0) {
            self.move_to(self.label("Types"));
        }
        if self.at(27.8) {
            self.click_at = Some(self.index);
        }
        if self.at(28.4) {
            self.move_to(pos2(1160.0, 280.0));
        }
        if self.at(33.5) {
            self.move_to(self.label("Map"));
        }
        if self.at(34.3) {
            self.click_at = Some(self.index);
        }
        if self.at(35.0) {
            let point = self
                .labels
                .iter()
                .find(|(text, rect)| text == "Media" && rect.top() > 700.0)
                .expect("media legend")
                .1
                .center();
            self.move_to(point);
        }
        if self.at(35.8) {
            self.click_at = Some(self.index);
        }
        if self.at(36.5) {
            self.move_to(ui::point(&self.app, self.targets["showreel"]));
        }
        if self.at(39.3) {
            self.move_to(self.label("All types"));
        }
        if self.at(40.1) {
            self.click_at = Some(self.index);
        }
        if self.at(40.7) {
            self.move_to(pos2(920.0, 90.0));
        }

        if let Some((start, from, to)) = self.movement {
            let t = ((self.index - start) as f32 / (self.fps as f32 * 0.65)).clamp(0.0, 1.0);
            let ease = t * t * (3.0 - 2.0 * t);
            self.position = from + (to - from) * ease;
        }
        input.events.push(egui::Event::PointerMoved(self.position));
        if let Some(at) = self.click_at
            && (self.index == at || self.index == at + 1)
        {
            input.events.push(egui::Event::PointerButton {
                pos: self.position,
                button: egui::PointerButton::Primary,
                pressed: self.index == at,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn cursor(&self, ctx: &egui::Context) {
        let p = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("demo-cursor"),
        ));
        if let Some(at) = self.click_at {
            let age = (self.index.saturating_sub(at)) as f32 / self.fps as f32;
            if age < 0.45 {
                p.circle_stroke(
                    self.position,
                    8.0 + age * 46.0,
                    Stroke::new(
                        2.0_f32,
                        Color32::from_rgba_unmultiplied(
                            139,
                            229,
                            208,
                            ((1.0 - age / 0.45) * 200.0) as u8,
                        ),
                    ),
                );
            }
        }
        let points = [
            vec2(0., 0.),
            vec2(0., 20.),
            vec2(5.5, 15.),
            vec2(10., 25.),
            vec2(14., 23.),
            vec2(9.5, 13.),
            vec2(17., 13.),
        ]
        .map(|v| self.position + v)
        .to_vec();
        p.add(egui::Shape::convex_polygon(
            points,
            Color32::WHITE,
            Stroke::new(1.5_f32, Color32::from_rgb(8, 12, 16)),
        ));
    }
    fn verify(&mut self) {
        for (sec, view, scope, query, category) in [
            (5., "Map", 0, "", None),
            (11., "Map", self.targets["media"], "", None),
            (18., "Files", 0, "", None),
            (25., "Files", 0, "ext:mp4 >1GB", None),
            (30., "Types", 0, "", None),
            (38., "Map", 0, "", Some("Media")),
            (41., "Map", 0, "", None),
        ] {
            if self.at(sec) {
                let state = ui::state(&self.app);
                assert_eq!(state["view"], view);
                assert_eq!(state["scope"], scope);
                assert_eq!(state["query"], query);
                assert_eq!(state["category"].as_str(), category);
                if view == "Files" {
                    assert!(state["rows"].as_u64().unwrap() > 0);
                }
                self.milestones
                    .push(serde_json::json!({"time":sec,"state":state}));
            }
        }
    }
}

impl eframe::App for Capture {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|event| matches!(event, egui::Event::Screenshot { .. }));
        input.time = Some(self.index as f64 / self.fps as f64);
        if self.started && !self.pending {
            self.script(input);
        } else {
            input.events.push(egui::Event::PointerMoved(self.position));
        }
    }
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        let received = screenshot.is_some();
        if let Some(image) = screenshot {
            assert_eq!(
                image.size,
                [1600, 900],
                "capture must contain only the fixed app framebuffer"
            );
            let mut rgb = Vec::with_capacity(image.pixels.len() * 3);
            for pixel in &image.pixels {
                rgb.extend_from_slice(&pixel.to_array()[..3]);
            }
            self.pipe
                .as_mut()
                .unwrap()
                .write_all(&rgb)
                .expect("write video frame");
            if self.index.is_multiple_of(self.fps * 2) {
                let mut ppm = b"P6\n1600 900\n255\n".to_vec();
                ppm.extend_from_slice(&rgb);
                std::fs::write(self.out.join(format!("frame-{:04}.ppm", self.index)), ppm).unwrap();
            }
            self.index += 1;
            self.pending = false;
            if self.index == self.total {
                self.pipe.take();
                let result = self.encoder.as_mut().unwrap().wait().unwrap();
                assert!(result.success());
                let report = serde_json::json!({"frames":self.index,"fps":self.fps,"size":[1600,900],"capture":"egui framebuffer only",
                    "data":"all generated; no live scan or disk actions","milestones":self.milestones,"painted_text":self.texts});
                std::fs::write(
                    self.out.join("capture-manifest.json"),
                    serde_json::to_vec_pretty(&report).unwrap(),
                )
                .unwrap();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        if !self.started {
            self.app.update(ctx, _frame);
            if ui::ready(&self.app) && !painted_text(ctx).is_empty() {
                ui::assert_no_io(&self.app);
                let mut encoder = Command::new(&self.ffmpeg)
                    .args([
                        "-y",
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        "rgb24",
                        "-s",
                        "1600x900",
                        "-r",
                        &self.fps.to_string(),
                        "-i",
                        "-",
                        "-an",
                        "-c:v",
                        "libx264",
                        "-preset",
                        "fast",
                        "-crf",
                        "16",
                        "-pix_fmt",
                        "yuv420p",
                    ])
                    .arg(self.out.join("ui-capture.mp4"))
                    .stdin(Stdio::piped())
                    .spawn()
                    .unwrap();
                self.pipe = encoder.stdin.take();
                self.encoder = Some(encoder);
                self.started = true;
                self.labels = painted_text(ctx);
            }
            ctx.request_repaint();
            return;
        }
        self.app.update(ctx, _frame);
        ui::assert_no_io(&self.app);
        self.labels = painted_text(ctx);
        for (text, _) in &self.labels {
            let lower = text.to_lowercase();
            assert!(
                !["c:\\users\\", "c:/users/", "\\appdata\\", "/appdata/"]
                    .iter()
                    .any(|s| lower.contains(s)),
                "Personal text rejected"
            );
            self.texts.insert(text.clone());
        }
        self.cursor(ctx);
        if self.pending || received {
            ctx.request_repaint();
            return;
        }
        self.verify();
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        self.pending = true;
        ctx.request_repaint();
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.windows(2)
            .find(|p| p[0] == name)
            .map(|p| p[1].clone())
            .unwrap_or_else(|| panic!("Required {name}"))
    };
    assert!(
        !args
            .iter()
            .any(|v| matches!(v.as_str(), "--auto-scan" | "--scan" | "--mft-file"))
    );
    #[cfg(windows)]
    assert_eq!(
        unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() },
        0,
        "Standard token required for demo capture"
    );
    let snapshot: canopy::model::Scan =
        serde_json::from_slice(&std::fs::read(arg("--snapshot")).unwrap()).unwrap();
    assert_eq!(snapshot.root.to_str(), Some("Z:\\"));
    assert!(
        snapshot
            .stats
            .notices
            .iter()
            .any(|n| n == "DISKVALE_SYNTHETIC_DEMO_ONLY")
    );
    let out = PathBuf::from(arg("--output"));
    std::fs::create_dir_all(&out).unwrap();
    let targets =
        serde_json::from_slice(&std::fs::read(out.join("targets.json")).unwrap()).unwrap();
    let fps = args
        .windows(2)
        .find(|p| p[0] == "--fps")
        .map_or(30, |p| p[1].parse::<usize>().unwrap());
    let ffmpeg = PathBuf::from(arg("--ffmpeg"));
    eframe::run_native(
        "Diskvale — Demo data",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1600., 900.])
                .with_min_inner_size([1600., 900.])
                .with_max_inner_size([1600., 900.])
                .with_position([40., 40.])
                .with_resizable(false)
                .with_icon(ui::brand::window_icon()),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| {
            cc.egui_ctx.set_pixels_per_point(1.0);
            let mut app = ui::Canopy::new(cc);
            ui::restrict_to_demo(&mut app);
            Ok(Box::new(Capture {
                app,
                out,
                targets,
                fps,
                total: fps * 42,
                index: 0,
                pending: false,
                encoder: None,
                pipe: None,
                ffmpeg,
                position: pos2(900., 90.),
                movement: None,
                click_at: None,
                labels: vec![],
                texts: BTreeSet::new(),
                milestones: vec![],
                started: false,
            }))
        }),
    )
}
