use canopy::{
    file_types::Category,
    model::{Metric, Scan, Units, count},
    query::{self, Query},
    recycle,
    scanner::{self, Progress},
    treemap,
};
use eframe::egui::{self, Color32, FontId, Rect, RichText, Stroke, vec2};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

#[path = "ui/brand.rs"]
pub(crate) mod brand;

const BG: Color32 = Color32::from_rgb(17, 22, 27);
const PANEL: Color32 = Color32::from_rgb(23, 29, 35);
const RAISED: Color32 = Color32::from_rgb(29, 37, 43);
const TEXT: Color32 = Color32::from_rgb(230, 237, 234);
const TILE_INK: Color32 = Color32::from_rgb(8, 12, 16);
const MUTED: Color32 = Color32::from_rgb(143, 160, 168);
const ACCENT: Color32 = Color32::from_rgb(202, 235, 121);
const BORDER: Color32 = Color32::from_rgb(44, 55, 62);
const GROUPED: Color32 = Color32::from_rgb(43, 54, 65);
const AMBER: Color32 = Color32::from_rgb(230, 159, 55);
const FOLDER: Color32 = Color32::from_rgb(184, 197, 205);
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Folders,
    Largest,
    Types,
}

/// One line of the folder tree. `More` stands in for children the tree does not list.
#[derive(Clone, Copy)]
enum TreeRow {
    Entry {
        id: usize,
        depth: usize,
    },
    More {
        parent: usize,
        depth: usize,
        hidden: usize,
    },
}

#[derive(Clone)]
struct DeleteConfirmation {
    target: recycle::Target,
    method: recycle::Method,
    focus_cancel: bool,
}

impl DeleteConfirmation {
    fn new(target: recycle::Target, method: recycle::Method) -> Self {
        Self {
            target,
            method,
            focus_cancel: true,
        }
    }
}

struct DeleteJob {
    target: recycle::Target,
    method: recycle::Method,
    receiver: mpsc::Receiver<Result<recycle::Outcome, String>>,
}

pub struct Canopy {
    path: String,
    scan: Option<Arc<Scan>>,
    current: usize,
    selected: Option<usize>,
    metric: Metric,
    view: View,
    query: String,
    rows: Arc<[usize]>,
    dirty: bool,
    rows_cache: results::ResultCache<results::RowsKey, Arc<[usize]>>,
    extensions_cache: results::ResultCache<results::TypesKey, results::TypeTotals>,
    prefetch_results: bool,
    descending: bool,
    progress: Arc<Progress>,
    receiver: Option<mpsc::Receiver<Result<Scan, String>>>,
    started: Instant,
    message: String,
    scan_error: Option<String>,
    mft_file: Option<PathBuf>,
    baseline: Option<Arc<Scan>>,
    baseline_map: HashMap<String, (u64, u64)>,
    map_cache: Vec<treemap::Tile>,
    map_shapes: Vec<egui::Shape>,
    map_key: Option<(usize, Metric, i32, i32, i32, i32, i32)>,
    units: Units,
    category: Option<Category>,
    depth: u16,
    minimum: u64,
    weights: Option<Vec<u64>>,
    matching_files: Option<Vec<u64>>,
    filter_key: Option<(Metric, String, Option<Category>)>,
    filter_receiver: Option<mpsc::Receiver<query::MapTotals>>,
    filtering_key: Option<(Metric, String, Option<Category>)>,
    expanded: HashSet<usize>,
    tree_rows: Vec<TreeRow>,
    tree_dirty: bool,
    tree_selection: Option<usize>,
    scroll_selection: bool,
    show_zero_entries: bool,
    search_focus: bool,
    loaded_at: Option<Instant>,
    elevated: bool,
    initial_focus: Option<String>,
    types_cache: results::TypeTotals,
    types_key: Option<(usize, Metric)>,
    /// Map entry under the pointer when the context menu was opened.
    map_menu: Option<usize>,
    map_menu_is_group: bool,
    delete_pending: Option<DeleteConfirmation>,
    deletion: Option<DeleteJob>,
    scan_stale: bool,
    /// Files view is ordered by modified date rather than by size.
    by_modified: bool,
    #[cfg(windows)]
    drives: Vec<scanner::windows::Drive>,
}

impl Canopy {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_ui(&cc.egui_ctx);
        let args: Vec<_> = std::env::args().collect();
        #[cfg(windows)]
        let drives = scanner::windows::drives();
        let path = args
            .iter()
            .position(|a| a == "--path")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| {
                #[cfg(windows)]
                if let Some(drive) = drives
                    .iter()
                    .find(|drive| drive.filesystem.eq_ignore_ascii_case("NTFS"))
                {
                    return drive.root.display().to_string();
                }
                "C:\\".into()
            });
        let mut app = Self {
            path,
            scan: None,
            current: 0,
            selected: None,
            metric: Metric::Allocated,
            view: match args
                .iter()
                .position(|arg| arg == "--view")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
            {
                Some("largest") => View::Largest,
                Some("types") => View::Types,
                _ => View::Folders,
            },
            query: String::new(),
            rows: vec![].into(),
            dirty: true,
            rows_cache: results::ResultCache::default(),
            extensions_cache: results::ResultCache::default(),
            prefetch_results: true,
            descending: true,
            progress: Arc::new(Progress::default()),
            receiver: None,
            started: Instant::now(),
            message: String::new(),
            scan_error: None,
            mft_file: args
                .iter()
                .position(|a| a == "--mft-file")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from),
            baseline: None,
            baseline_map: HashMap::new(),
            map_cache: vec![],
            map_shapes: vec![],
            map_key: None,
            units: Units::default(),
            category: None,
            depth: 96,
            minimum: 0,
            weights: None,
            matching_files: None,
            filter_key: None,
            filter_receiver: None,
            filtering_key: None,
            expanded: HashSet::from([0]),
            tree_rows: vec![],
            tree_dirty: true,
            tree_selection: None,
            scroll_selection: false,
            show_zero_entries: false,
            search_focus: false,
            loaded_at: None,
            elevated: layout::is_elevated(),
            initial_focus: args
                .iter()
                .position(|arg| arg == "--focus")
                .and_then(|i| args.get(i + 1))
                .cloned(),
            types_cache: vec![].into(),
            types_key: None,
            map_menu: None,
            map_menu_is_group: false,
            delete_pending: None,
            deletion: None,
            scan_stale: false,
            by_modified: false,
            #[cfg(windows)]
            drives,
        };
        if let Some(file) = args
            .iter()
            .position(|a| a == "--snapshot")
            .and_then(|i| args.get(i + 1))
        {
            app.load_scan(cc.egui_ctx.clone(), PathBuf::from(file));
        } else if args.iter().any(|a| a == "--auto-scan") {
            app.start_scan(cc.egui_ctx.clone());
        }
        app
    }
    fn load_scan(&mut self, ctx: egui::Context, path: PathBuf) {
        if self.deletion.is_some() || self.delete_pending.is_some() {
            return;
        }
        self.scan_error = None;
        self.progress = Arc::new(Progress::default());
        self.progress.phase.store(3, Ordering::Relaxed);
        self.started = Instant::now();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let progress = self.progress.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                let file = std::fs::File::open(&path)
                    .map_err(|e| format!("Could not open snapshot: {e}"))?;
                let reader = CancellableReader {
                    inner: file,
                    progress: progress.clone(),
                };
                let mut scan: Scan = serde_json::from_reader(std::io::BufReader::new(reader))
                    .map_err(|e| format!("Could not read snapshot: {e}"))?;
                if !valid_snapshot(&scan) {
                    return Err("Invalid snapshot tree.".into());
                }
                if progress.cancelled() {
                    return Err("Snapshot loading cancelled.".into());
                }
                scan.stats.notices.insert(
                    0,
                    "Loaded a saved scan. The counts and scan time refer to that captured run."
                        .into(),
                );
                Ok(scan)
            })
            .unwrap_or_else(|_| Err("Could not load snapshot.".into()));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    fn start_scan(&mut self, ctx: egui::Context) {
        if self.deletion.is_some() || self.delete_pending.is_some() {
            return;
        }
        if self.receiver.is_some() {
            return;
        }
        let root = PathBuf::from(self.path.trim());
        self.scan_error = None;
        if root.as_os_str().is_empty() {
            self.message = "Choose a whole NTFS drive.".into();
            return;
        }
        self.progress = Arc::new(Progress::default());
        self.started = Instant::now();
        self.message.clear();
        let progress = self.progress.clone();
        let mft_file = self.mft_file.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| run_ui_scan(root, progress, mft_file))
                .unwrap_or_else(|_| {
                    Err("The scanner stopped unexpectedly. Please rescan the NTFS drive.".into())
                });
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    fn poll(&mut self) {
        self.rows_cache.poll();
        self.extensions_cache.poll();
        let deleted = self
            .deletion
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "The delete operation disconnected. Rescan to check the current files.".into(),
                )),
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = deleted {
            let job = self.deletion.take().unwrap();
            self.scan_stale = true;
            match result {
                Ok(recycle::Outcome::Deleted) => {
                    self.scan_error = None;
                    self.message = match job.method {
                        recycle::Method::RecycleBin => format!(
                            "Moved {} to the Recycle Bin. Results are out of date; Rescan to refresh. Recycled items still occupy disk space.",
                            job.target.path.display()
                        ),
                        recycle::Method::Permanent => format!(
                            "Permanently deleted {}. Results are out of date; Rescan to refresh.",
                            job.target.path.display()
                        ),
                    };
                }
                Ok(recycle::Outcome::CannotRecycle)
                    if job.method == recycle::Method::RecycleBin =>
                {
                    self.scan_error = None;
                    self.message.clear();
                    self.delete_pending = Some(DeleteConfirmation::new(
                        job.target,
                        recycle::Method::Permanent,
                    ));
                }
                Ok(recycle::Outcome::CannotRecycle) => {
                    self.scan_error = Some("Windows did not finish deleting this item. Rescan to check the current files.".into());
                }
                Err(error) => self.scan_error = Some(error),
            }
        }
        let outcome = self.receiver.as_ref().and_then(|r| match r.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("Scanner disconnected.".into())),
            _ => None,
        });
        if let Some(outcome) = outcome {
            self.receiver = None;
            match outcome {
                Ok(scan) => {
                    self.path = scan.root.display().to_string();
                    self.current = 0;
                    self.selected = None;
                    if let Some(folder) = self.initial_focus.take() {
                        if let Some(id) = focus_entry(&scan, &folder) {
                            if scan.nodes[id].directory {
                                self.current = id;
                            } else {
                                self.current = scan.nodes[id].parent.unwrap_or(0);
                                self.selected = Some(id);
                            }
                        } else {
                            self.message = format!("Folder not found in this scan: {folder}");
                        }
                    }
                    self.scan = Some(Arc::new(scan));
                    self.scan_stale = false;
                    self.delete_pending = None;
                    self.loaded_at = Some(Instant::now());
                    self.expanded = HashSet::from([0]);
                    self.tree_dirty = true;
                    self.tree_selection = None;
                    self.rows_cache = results::ResultCache::default();
                    self.extensions_cache = results::ResultCache::default();
                    self.prefetch_results = true;
                    self.rows = vec![].into();
                    self.types_cache = vec![].into();
                    self.weights = None;
                    self.matching_files = None;
                    self.filter_key = None;
                    self.filter_receiver = None;
                    self.filtering_key = None;
                    self.dirty = true;
                    self.map_key = None;
                    self.types_key = None;
                    if self
                        .baseline
                        .as_ref()
                        .is_some_and(|b| self.scan.as_ref().is_some_and(|s| b.root != s.root))
                    {
                        self.baseline = None;
                        self.baseline_map.clear();
                    }
                }
                Err(error) => self.scan_error = Some(error),
            }
        }
    }
    fn navigate(&mut self, id: usize) {
        self.current = id;
        self.expanded.insert(id);
        self.tree_dirty = true;
        self.select(id);
        self.scroll_selection = true;
        // The search and type filter stay on, so zooming into a result keeps its context.
        self.dirty = true;
        self.map_key = None;
        self.types_key = None;
    }
    fn rows_key(&self) -> results::RowsKey {
        results::RowsKey {
            scope: self.current,
            metric: self.metric,
            query: self.query.to_lowercase(),
            category: self.category,
            by_modified: self.by_modified,
            descending: self.descending,
        }
    }
    fn rebuild_rows(&mut self, ctx: &egui::Context) -> bool {
        let Some(scan) = self.scan.clone() else {
            return false;
        };
        let key = self.rows_key();
        let build_key = key.clone();
        let background = scan.nodes.len() > 2048;
        self.dirty = false;
        if let Some(rows) = self.rows_cache.request(key, background, ctx, move || {
            results::file_rows(&scan, &build_key)
        }) {
            self.rows = rows;
            true
        } else {
            false
        }
    }
    fn type_results(&mut self, ctx: &egui::Context, filtered: bool) -> Option<results::TypeTotals> {
        let scan = self.scan.clone()?;
        let key = results::TypesKey {
            scope: self.current,
            metric: self.metric,
            query: if filtered {
                self.query.clone()
            } else {
                String::new()
            },
            category: if filtered { self.category } else { None },
        };
        let build_key = key.clone();
        let background = scan.nodes.len() > 2048;
        self.extensions_cache
            .request(key, background, ctx, move || {
                results::type_totals(&scan, &build_key)
            })
    }
    fn loading_table(ui: &mut egui::Ui, height: f32, label: &str) {
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), height + 26.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.add_space((height * 0.35).max(12.0));
                ui.spinner();
                ui.label(RichText::new(label).color(MUTED));
            },
        );
    }
    fn delta(&self, scan: &Scan, id: usize) -> Option<i128> {
        self.baseline.as_ref()?;
        let old = self
            .baseline_map
            .get(&scan.relative_path(id))
            .copied()
            .unwrap_or_default();
        let old = match self.metric {
            Metric::Allocated => old.1,
            Metric::Logical => old.0,
        };
        Some(scan.nodes[id].size(self.metric) as i128 - old as i128)
    }
    fn table(&mut self, ui: &mut egui::Ui, scan: &Arc<Scan>, height: f32) {
        use egui_extras::{Column, TableBuilder};
        if self.view == View::Types {
            let Some(totals) = self.type_results(ui.ctx(), true) else {
                Self::loading_table(ui, height, "Preparing file types…");
                return;
            };
            let mut types: Vec<_> = totals.iter().collect();
            if !self.descending {
                types.reverse();
            }
            if types.is_empty() {
                ui.add_sized(
                    [ui.available_width(), height + 26.0],
                    egui::Label::new(
                        RichText::new("No file types match this search.").color(MUTED),
                    ),
                );
                return;
            }
            let share_width = if ui.available_width() < 900.0 {
                140.0
            } else {
                230.0
            };
            TableBuilder::new(ui)
                .id_salt("file-types")
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::remainder().at_least(180.0))
                .column(Column::initial(110.0))
                .column(Column::initial(120.0))
                .column(Column::initial(110.0))
                .column(Column::initial(share_width))
                .min_scrolled_height(height)
                .max_scroll_height(height)
                .auto_shrink([false, false])
                .header(26.0, |mut header| {
                    for title in [
                        "FILE TYPE",
                        "CATEGORY",
                        if self.metric == Metric::Allocated {
                            "SIZE ON DISK"
                        } else {
                            "FILE SIZE"
                        },
                        "FILES",
                        "SHARE OF FOLDER",
                    ] {
                        header.col(|ui| {
                            ui.label(RichText::new(title).size(12.0).color(MUTED).strong());
                        });
                    }
                })
                .body(|body| {
                    body.rows(29.0, types.len(), |mut row| {
                        let (ext, size, files) = types[row.index()];
                        row.col(|ui| {
                            entry_icon(ui, false, color(ext));
                            ui.add(egui::Label::new(RichText::new(ext).strong()).truncate());
                        });
                        row.col(|ui| {
                            let category = Category::from_extension(ext);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(category.label()).size(12.0).color(MUTED),
                                )
                                .truncate(),
                            )
                            .on_hover_text(category.examples());
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(self.size(*size)).monospace().size(13.0));
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(count(*files)).monospace().size(13.0));
                        });
                        row.col(|ui| {
                            let share = *size as f32
                                / scan.nodes[self.current].size(self.metric).max(1) as f32;
                            ui.add(
                                egui::ProgressBar::new(share)
                                    .text(format!("{:.1}%", share * 100.0))
                                    .fill(color(ext))
                                    .desired_width(ui.available_width()),
                            );
                        });
                    });
                });
            return;
        }
        if !self.rebuild_rows(ui.ctx()) {
            Self::loading_table(ui, height, "Preparing file list…");
            return;
        }
        let rows = self.rows.clone();
        if rows.is_empty() {
            ui.add_sized(
                [ui.available_width(), height + 26.0],
                egui::Label::new(
                    RichText::new(if self.query.is_empty() {
                        "This folder has no entries."
                    } else {
                        "No entries match this search."
                    })
                    .color(MUTED),
                ),
            );
            return;
        }
        let denominator = scan.nodes[self.current].size(self.metric).max(1) as f32;
        let size_width = if ui.available_width() < 900.0 {
            108.0
        } else {
            126.0
        };
        TableBuilder::new(ui)
            .id_salt("entries")
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::remainder().at_least(180.0))
            .column(Column::initial(size_width))
            .column(Column::initial(size_width))
            .column(Column::initial(112.0))
            .column(Column::initial(92.0))
            .column(Column::initial(if self.baseline.is_some() {
                135.0
            } else {
                90.0
            }))
            .min_scrolled_height(height)
            .max_scroll_height(height)
            .auto_shrink([false, false])
            .header(26.0, |mut header| {
                for (index, title) in [
                    "NAME / LOCATION",
                    "FILE SIZE",
                    "SIZE ON DISK",
                    "SHARE OF FOLDER",
                    "MODIFIED",
                    if self.baseline.is_some() {
                        "CHANGE"
                    } else {
                        "FILES"
                    },
                ]
                .into_iter()
                .enumerate()
                {
                    header.col(|ui| {
                        let sort_metric = match index {
                            1 => Some(Metric::Logical),
                            2 => Some(Metric::Allocated),
                            _ => None,
                        };
                        if index == 4 {
                            // Newest first on the first click, like the size columns.
                            let active = self.by_modified;
                            let arrow = match (active, self.descending) {
                                (true, true) => " ↓",
                                (true, false) => " ↑",
                                _ => "",
                            };
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(format!("{title}{arrow}"))
                                            .size(12.0)
                                            .color(if active { TEXT } else { MUTED })
                                            .strong(),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text("Sort by date modified (UTC)")
                                .clicked()
                            {
                                self.descending = if active { !self.descending } else { true };
                                self.by_modified = true;
                                self.dirty = true;
                            }
                        } else if let Some(metric) = sort_metric {
                            let active = self.metric == metric && !self.by_modified;
                            let arrow = match (active, self.descending) {
                                (true, true) => " ↓",
                                (true, false) => " ↑",
                                _ => "",
                            };
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(format!("{title}{arrow}"))
                                            .size(12.0)
                                            .color(if active { TEXT } else { MUTED })
                                            .strong(),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text("Sort by this size")
                                .clicked()
                            {
                                self.descending = if active { !self.descending } else { true };
                                self.by_modified = false;
                                self.metric = metric;
                                self.tree_dirty = true;
                                self.dirty = true;
                                self.map_key = None;
                            }
                        } else {
                            ui.label(RichText::new(title).size(12.0).color(MUTED).strong());
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(42.0, rows.len(), |mut row| {
                    let id = rows[row.index()];
                    let node = &scan.nodes[id];
                    row.set_selected(self.selected == Some(id));
                    row.col(|ui| {
                        entry_icon(
                            ui,
                            node.directory,
                            if node.directory {
                                FOLDER
                            } else {
                                file_color(&node.name)
                            },
                        );
                        let text = if node.directory {
                            RichText::new(&node.name).strong()
                        } else {
                            RichText::new(&node.name)
                        };
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(text).truncate())
                                .on_hover_text(scan.path(id).display().to_string());
                            let parent = scan.relative_path(id);
                            let parent = parent.rsplit_once(['\\', '/']).map_or("", |(p, _)| p);
                            ui.add(
                                egui::Label::new(RichText::new(parent).size(12.0).color(MUTED))
                                    .truncate(),
                            );
                        });
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(self.size(node.logical))
                                .monospace()
                                .size(13.0),
                        );
                    });
                    row.col(|ui| {
                        let allocated = if node.shared {
                            "Shared link".into()
                        } else if node.allocation_unknown {
                            format!("{}*", self.size(node.allocated))
                        } else {
                            self.size(node.allocated)
                        };
                        ui.label(RichText::new(allocated).monospace().size(13.0));
                    });
                    row.col(|ui| {
                        let share = node.size(self.metric) as f32 / denominator;
                        ui.add(
                            egui::ProgressBar::new(share)
                                .text(format!("{:.1}%", share * 100.0))
                                .fill(Color32::from_rgb(73, 102, 120))
                                .desired_width(100.0),
                        );
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(layout::modified_day(node.modified))
                                .monospace()
                                .size(13.0)
                                .color(MUTED),
                        )
                        .on_hover_text("Date modified, UTC");
                    });
                    row.col(|ui| {
                        if let Some(delta) = self.delta(scan, id) {
                            let text = if delta == 0 {
                                "—".into()
                            } else {
                                format!(
                                    "{}{}",
                                    if delta > 0 { "+" } else { "−" },
                                    self.size(delta.unsigned_abs().min(u64::MAX as u128) as u64)
                                )
                            };
                            ui.colored_label(
                                if delta > 0 {
                                    Color32::from_rgb(233, 174, 105)
                                } else {
                                    ACCENT
                                },
                                text,
                            );
                        } else {
                            ui.label(RichText::new(count(node.files)).monospace().size(13.0));
                        }
                    });
                    let response = row.response();
                    if response.clicked() {
                        self.select(id);
                    }
                    if response.double_clicked() && node.directory {
                        self.navigate(id);
                    }
                    response.context_menu(|ui| {
                        if node.directory && ui.button("Zoom in").clicked() {
                            self.navigate(id);
                            ui.close();
                        }
                        if ui.button("Show in Explorer").clicked() {
                            reveal(&scan.path(id));
                            ui.close();
                        }
                        if ui.button("Copy path").clicked() {
                            ui.ctx().copy_text(scan.path(id).display().to_string());
                            ui.close();
                        }
                        self.delete_menu_item(ui, scan, id, false);
                    });
                });
            });
    }

    fn save_snapshot(&mut self, scan: &Scan) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Diskvale snapshot", &["json"])
            .set_file_name("diskvale-snapshot.json")
            .save_file()
        {
            self.message = match write_snapshot(scan, &path) {
                Ok(_) => format!("Snapshot saved to {}", path.display()),
                Err(e) => format!("Could not save snapshot: {e}"),
            };
        }
    }
    fn load_baseline(&mut self, scan: &Scan) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Diskvale snapshot", &["json"])
            .pick_file()
        {
            let loaded = std::fs::File::open(path)
                .map_err(|e| e.to_string())
                .and_then(|f| {
                    serde_json::from_reader::<_, Scan>(std::io::BufReader::new(f))
                        .map_err(|e| e.to_string())
                });
            match loaded {
                Ok(old) if old.root == scan.root && valid_snapshot(&old) => {
                    self.baseline_map = (0..old.nodes.len())
                        .map(|i| {
                            (
                                old.relative_path(i),
                                (old.nodes[i].logical, old.nodes[i].allocated),
                            )
                        })
                        .collect();
                    self.baseline = Some(Arc::new(old));
                    self.message =
                        "Comparison loaded. Change values use the selected size metric.".into();
                }
                Ok(_) => {
                    self.message = "Choose a valid snapshot of the same folder or drive.".into()
                }
                Err(e) => self.message = format!("Could not load snapshot: {e}"),
            }
        }
    }
    fn export_csv(&mut self, scan: &Scan) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("diskvale-files.csv")
            .save_file()
        {
            self.message = match write_csv(scan, &path) {
                Ok(_) => format!("Export saved to {}", path.display()),
                Err(e) => format!("Could not export: {e}"),
            };
        }
    }
}

#[path = "ui/layout.rs"]
mod layout;
#[path = "ui/results.rs"]
mod results;

impl eframe::App for Canopy {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.progress.cancel.store(true, Ordering::Relaxed);
    }
}
fn configure_ui(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BG;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = Color32::from_rgb(42, 54, 65);
    visuals.selection.stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.inactive.weak_bg_fill = RAISED;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 55, 62);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(42, 55, 62);
    visuals.widgets.active.bg_fill = Color32::from_rgb(48, 65, 80);
    ctx.set_visuals(visuals);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = vec2(8.0, 4.0);
    style.spacing.button_padding = vec2(10.0, 5.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(14.0));
    ctx.set_style(style);
}

struct CancellableReader<R> {
    inner: R,
    progress: Arc<Progress>,
}

impl<R: std::io::Read> std::io::Read for CancellableReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.progress.cancelled() {
            return Err(std::io::Error::other("Snapshot loading cancelled."));
        }
        self.inner.read(buffer)
    }
}

fn is_saved_scan(scan: &Scan) -> bool {
    scan.stats
        .notices
        .iter()
        .any(|notice| notice.starts_with("Loaded a saved scan."))
}

// Launch directly into a folder of a captured or newly completed scan.
// Resolve only inside the indexed tree; never follow filesystem targets.
fn focus_entry(scan: &Scan, relative: &str) -> Option<usize> {
    let mut current = 0;
    for name in relative.split(['\\', '/']).filter(|name| !name.is_empty()) {
        if matches!(name, "." | "..") {
            return None;
        }
        if !scan.nodes[current].directory {
            return None;
        }
        current = *scan.nodes[current]
            .children
            .iter()
            .find(|&&id| scan.nodes[id].name.eq_ignore_ascii_case(name))?;
    }
    Some(current)
}

fn color(name: &str) -> Color32 {
    category_color(Category::from_extension(name))
}
fn file_color(name: &str) -> Color32 {
    category_color(Category::from_name(name))
}
fn category_color(category: Category) -> Color32 {
    let [r, g, b] = category.rgb();
    Color32::from_rgb(r, g, b)
}
fn luminance(color: Color32) -> f32 {
    let linear = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}
fn contrast(a: Color32, b: Color32) -> f32 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn tile_ink(fill: Color32) -> Color32 {
    if contrast(fill, TILE_INK) >= contrast(fill, Color32::WHITE) {
        TILE_INK
    } else {
        Color32::WHITE
    }
}
fn entry_icon(ui: &mut egui::Ui, directory: bool, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
    if directory {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(1.0, 2.0), vec2(6.0, 4.0)),
            1,
            color,
        );
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(1.0, 5.0), vec2(12.0, 8.0)),
            2,
            color.gamma_multiply(0.85),
        );
    } else {
        ui.painter().circle_filled(rect.center(), 3.0, color);
    }
}
fn reveal(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = scanner::windows::reveal(path);
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}
fn run_ui_scan(
    root: PathBuf,
    progress: Arc<Progress>,
    mft_file: Option<PathBuf>,
) -> Result<Scan, String> {
    if let Some(file) = mft_file {
        scanner::mft::scan_file(&root, &file, &progress)
    } else {
        scanner::scan_mft_only(root, progress).map_err(|error| {
            format!(
                "MFT scan unavailable: {}. This scan did not complete.",
                error.trim_end_matches('.')
            )
        })
    }
}

fn valid_snapshot(scan: &Scan) -> bool {
    if scan.nodes.is_empty() || scan.nodes[0].parent.is_some() {
        return false;
    }
    let mut seen = vec![false; scan.nodes.len()];
    let mut stack = vec![0];
    while let Some(id) = stack.pop() {
        if id >= scan.nodes.len() || seen[id] {
            return false;
        }
        seen[id] = true;
        for &child in &scan.nodes[id].children {
            if child >= scan.nodes.len() || scan.nodes[child].parent != Some(id) {
                return false;
            }
            stack.push(child);
        }
    }
    seen.iter().all(|s| *s)
}
fn write_csv(scan: &Scan, path: &std::path::Path) -> Result<(), String> {
    let mut writer = csv::Writer::from_path(path).map_err(|e| e.to_string())?;
    writer
        .write_record([
            "path",
            "directory",
            "logical_bytes",
            "allocated_bytes",
            "shared_link",
            "allocation_unknown",
            "files",
            "modified_unix_seconds",
        ])
        .map_err(|e| e.to_string())?;
    for (id, node) in scan.nodes.iter().enumerate() {
        writer
            .write_record([
                scan.path(id).display().to_string(),
                node.directory.to_string(),
                node.logical.to_string(),
                node.allocated.to_string(),
                node.shared.to_string(),
                node.allocation_unknown.to_string(),
                node.files.to_string(),
                node.modified.to_string(),
            ])
            .map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())
}
fn write_snapshot(scan: &Scan, path: &std::path::Path) -> Result<(), String> {
    use std::io::Write;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, scan).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}
pub fn cli() {
    let args: Vec<_> = std::env::args().collect();
    let get = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
    };
    let Some(root) = get("--scan") else {
        return;
    };
    let progress = Arc::new(Progress::default());
    let result = run_ui_scan(root.into(), progress, get("--mft-file").map(PathBuf::from));
    match result {
        Ok(scan) => {
            if let Some(path) = get("--json") {
                write_snapshot(&scan, &PathBuf::from(path)).expect("snapshot output");
            }
            if let Some(path) = get("--csv") {
                write_csv(&scan, &PathBuf::from(path)).expect("CSV output");
            }
            println!("{}",serde_json::to_string_pretty(&serde_json::json!({"root":scan.root,"logical_bytes":scan.nodes[0].logical,"allocated_bytes":scan.nodes[0].allocated,"stats":scan.stats})).unwrap());
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_cancellation_stops_reading_before_the_remainder() {
        use std::io::Read;
        let progress = Arc::new(Progress::default());
        let mut reader = CancellableReader {
            inner: std::io::Cursor::new(b"snapshot data"),
            progress: progress.clone(),
        };
        let mut buffer = [0; 4];
        assert_eq!(reader.read(&mut buffer).unwrap(), 4);
        progress.cancel.store(true, Ordering::Relaxed);
        assert!(
            reader
                .read(&mut buffer)
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        assert_eq!(reader.inner.position(), 4);
    }
    #[test]
    fn launch_focus_resolves_entries_without_escaping_the_scan() {
        use canopy::model::Node;
        let mut scan = Scan {
            root: "C:\\".into(),
            stats: Default::default(),
            nodes: vec![
                Node::new("C:\\".into(), None, true),
                Node::new("Projects".into(), Some(0), true),
                Node::new("café.rs".into(), Some(1), false),
            ],
        };
        scan.nodes[0].children = vec![1];
        scan.nodes[1].children = vec![2];
        assert_eq!(focus_entry(&scan, ""), Some(0));
        assert_eq!(focus_entry(&scan, "projects"), Some(1));
        assert_eq!(focus_entry(&scan, "Projects/café.rs"), Some(2));
        assert_eq!(scan.nodes[2].parent, Some(1));
        for invalid in [
            "../Projects",
            "Projects/..",
            "Projects/missing",
            "Projects/café.rs/child",
            "C:\\Projects",
        ] {
            assert_eq!(focus_entry(&scan, invalid), None, "{invalid}");
        }
    }
    #[cfg(windows)]
    #[test]
    fn default_scan_rejects_folders_without_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ordinary.txt"), b"fixture").unwrap();
        let progress = Arc::new(Progress::default());
        let error = run_ui_scan(dir.path().into(), progress.clone(), None).unwrap_err();
        assert!(error.contains("requires a whole NTFS drive"), "{error}");
        assert!(error.contains("This scan did not complete"), "{error}");
        assert_eq!(progress.files.load(Ordering::Relaxed), 0);
        assert_eq!(progress.folders.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn rejects_cyclic_snapshot() {
        let mut scan = Scan {
            root: "C:\\".into(),
            nodes: vec![canopy::model::Node::new("root".into(), None, true)],
            stats: Default::default(),
        };
        assert!(valid_snapshot(&scan));
        scan.nodes[0].children.push(0);
        assert!(!valid_snapshot(&scan));
    }
}
