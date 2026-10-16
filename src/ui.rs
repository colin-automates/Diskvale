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
pub mod brand;

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
        next_page: usize,
    },
}

#[derive(Clone)]
struct DeleteConfirmation {
    target: recycle::Target,
    method: recycle::Method,
    focus_cancel: bool,
    generation: u64,
}

impl DeleteConfirmation {
    fn new(target: recycle::Target, method: recycle::Method) -> Self {
        Self {
            target,
            method,
            focus_cancel: true,
            generation: 0,
        }
    }
}

struct DeleteJob {
    generation: u64,
    target: recycle::Target,
    method: recycle::Method,
    receiver: mpsc::Receiver<Result<recycle::Outcome, String>>,
}

enum IoResult {
    Saved(PathBuf),
    Exported(PathBuf),
    Baseline(Arc<Scan>, HashMap<String, (u64, u64)>),
}
struct IoJob {
    generation: u64,
    label: &'static str,
    progress: Arc<Progress>,
    receiver: mpsc::Receiver<Result<IoResult, String>>,
}

pub struct Canopy {
    io_job: Option<IoJob>,
    generation: u64,
    scan_request: u64,
    receiver_request: u64,
    close_popups: bool,
    tree_pages: HashMap<usize, usize>,
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
    receiver: Option<mpsc::Receiver<(u64, Result<Scan, String>)>>,
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
    #[cfg(windows)]
    drive_receiver: Option<mpsc::Receiver<Vec<scanner::windows::Drive>>>,
}

impl Canopy {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_ui(&cc.egui_ctx);
        let args: Vec<_> = std::env::args().collect();
        let path = args
            .iter()
            .position(|a| a == "--path")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| {
                std::env::var("SystemDrive")
                    .map(|drive| format!("{drive}\\"))
                    .unwrap_or_else(|_| "C:\\".into())
            });
        let mut app = Self {
            io_job: None,
            generation: 0,
            scan_request: 0,
            receiver_request: 0,
            close_popups: false,
            tree_pages: HashMap::new(),
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
            drives: vec![],
            #[cfg(windows)]
            drive_receiver: None,
        };
        #[cfg(windows)]
        app.refresh_drives(cc.egui_ctx.clone());
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
    #[cfg(windows)]
    fn refresh_drives(&mut self, ctx: egui::Context) {
        if self.drive_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.drive_receiver = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(scanner::windows::drives());
            ctx.request_repaint();
        });
    }
    fn load_scan(&mut self, ctx: egui::Context, path: PathBuf) {
        if self.receiver.is_some() || self.deletion.is_some() || self.delete_pending.is_some() {
            return;
        }
        self.scan_error = None;
        self.progress = Arc::new(Progress::default());
        self.progress.phase.store(3, Ordering::Relaxed);
        self.started = Instant::now();
        let (sender, receiver) = mpsc::channel();
        if let Some(scan) = &self.scan {
            scan.stats.runtime.mark_stale();
            self.scan_stale = true;
        }
        self.receiver = Some(receiver);
        self.scan_request += 1;
        self.receiver_request = self.scan_request;
        let request = self.scan_request;
        let progress = self.progress.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                let mut scan = read_snapshot(&path, progress.clone())?;
                scan.stats.notices.insert(
                    0,
                    "Loaded a saved scan. The counts and scan time refer to that captured run."
                        .into(),
                );
                Ok(scan)
            })
            .unwrap_or_else(|_| Err("Could not load snapshot.".into()));
            let _ = sender.send((request, result));
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
            self.message = "Choose a whole NTFS, exFAT or FAT32 drive.".into();
            return;
        }
        self.progress = Arc::new(Progress::default());
        self.started = Instant::now();
        self.message.clear();
        let progress = self.progress.clone();
        let mft_file = self.mft_file.clone();
        let (sender, receiver) = mpsc::channel();
        if let Some(scan) = &self.scan {
            scan.stats.runtime.mark_stale();
            self.scan_stale = true;
        }
        self.receiver = Some(receiver);
        self.scan_request += 1;
        self.receiver_request = self.scan_request;
        let request = self.scan_request;
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| run_ui_scan(root, progress, mft_file))
                .unwrap_or_else(|_| {
                    Err("The scanner stopped unexpectedly. Please rescan the drive.".into())
                });
            let _ = sender.send((request, result));
            ctx.request_repaint();
        });
    }
    fn poll(&mut self) {
        #[cfg(windows)]
        if let Some(outcome) =
            self.drive_receiver
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(drives) => Some(Ok(drives)),
                    Err(mpsc::TryRecvError::Disconnected) => Some(Err(())),
                    Err(mpsc::TryRecvError::Empty) => None,
                })
        {
            self.drive_receiver = None;
            match outcome {
                Ok(drives) => self.drives = drives,
                Err(()) => {
                    self.message = "Drive discovery stopped. Choose Refresh drives to retry.".into()
                }
            }
        }
        if let Some(outcome) = self
            .io_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Background output/comparison worker disconnected.".into(),
                )),
                Err(mpsc::TryRecvError::Empty) => None,
            })
        {
            let job = self.io_job.take().unwrap();
            self.message = match outcome {
                Ok(IoResult::Saved(path)) => format!(
                    "Snapshot from scan #{} saved to {}{}",
                    job.generation,
                    path.display(),
                    if job.generation != self.generation {
                        " (previous scan)"
                    } else {
                        ""
                    }
                ),
                Ok(IoResult::Exported(path)) => format!(
                    "Export from scan #{} saved to {}{}",
                    job.generation,
                    path.display(),
                    if job.generation != self.generation {
                        " (previous scan)"
                    } else {
                        ""
                    }
                ),
                Ok(IoResult::Baseline(old, map))
                    if job.generation == self.generation && !job.progress.cancelled() =>
                {
                    self.baseline = Some(old);
                    self.baseline_map = map;
                    "Comparison loaded. Change values use the selected size metric.".into()
                }
                Ok(IoResult::Baseline(_, _)) => {
                    "Comparison discarded because its scan changed or it was cancelled.".into()
                }
                Err(error) => format!("{}: {error}", job.label),
            };
        }
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
            if job.generation != self.generation {
                return;
            }
            self.scan_stale = true;
            // A recycle-only veto performed no mutation. Keep the exact target
            // eligible for the existing separately confirmed permanent action.
            if result != Ok(recycle::Outcome::CannotRecycle) {
                if let Some(scan) = &self.scan {
                    scan.stats.runtime.mark_stale();
                }
            } else {
                self.scan_stale = false;
            }
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
                    self.delete_pending.as_mut().unwrap().generation = self.generation;
                }
                Ok(recycle::Outcome::CannotRecycle) => {
                    self.scan_error = Some("Windows did not finish deleting this item. Rescan to check the current files.".into());
                }
                Err(error) => self.scan_error = Some(error),
            }
        }
        let outcome = self.receiver.as_ref().and_then(|r| match r.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some((self.receiver_request, Err("Scanner disconnected.".into())))
            }
            _ => None,
        });
        if let Some((request, outcome)) = outcome {
            self.receiver = None;
            if request != self.scan_request {
                return;
            }
            match outcome {
                Ok(scan) => {
                    self.install_scan(scan);
                }
                Err(error) => self.scan_error = Some(error),
            }
        }
    }

    fn install_scan(&mut self, scan: Scan) {
        if let Some(job) = &self.io_job {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(old) = &self.scan {
            old.stats.runtime.mark_stale();
        }
        self.generation += 1;
        self.close_popups = true;
        self.map_cache.clear();
        self.map_shapes.clear();
        self.map_menu = None;
        self.map_menu_is_group = false;
        self.tree_rows.clear();
        self.tree_pages.clear();
        self.scroll_selection = false;
        self.baseline = None;
        self.baseline_map.clear();
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
            .get(&comparison_key(scan, id))
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

    fn start_io(
        &mut self,
        ctx: &egui::Context,
        label: &'static str,
        work: impl FnOnce(Arc<Progress>) -> Result<IoResult, String> + Send + 'static,
    ) {
        if self.io_job.is_some() {
            return;
        }
        let progress = Arc::new(Progress::default());
        let (sender, receiver) = mpsc::sync_channel(1);
        self.io_job = Some(IoJob {
            generation: self.generation,
            label,
            progress: progress.clone(),
            receiver,
        });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(progress)))
                .unwrap_or_else(|_| Err("Background worker stopped unexpectedly.".into()));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    fn save_snapshot(&mut self, scan: &Arc<Scan>, ctx: &egui::Context) {
        if self.io_job.is_some() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Diskvale snapshot", &["json"])
            .set_file_name("diskvale-snapshot.json")
            .save_file()
        {
            let scan = scan.clone();
            self.start_io(ctx, "Saving snapshot", move |progress| {
                canopy::output::snapshot(&scan, &path, &progress)?;
                Ok(IoResult::Saved(path))
            });
        }
    }
    fn load_baseline(&mut self, scan: &Arc<Scan>, ctx: &egui::Context) {
        if self.io_job.is_some() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Diskvale snapshot", &["json"])
            .pick_file()
        {
            let scan = scan.clone();
            self.start_io(ctx, "Loading comparison", move |progress| {
                let old = read_snapshot(&path, progress.clone())?;
                if old.root != scan.root {
                    return Err("Choose a snapshot of the same drive.".into());
                }
                if !old.stats.filesystem.as_deref().unwrap_or("NTFS").eq_ignore_ascii_case(scan.stats.filesystem.as_deref().unwrap_or("NTFS")) {
                    return Err("Snapshot filesystems differ. Compare captures made with the same filesystem and scan method.".into());
                }
                if old.stats.directory_streams_captured != scan.stats.directory_streams_captured {
                    return Err("Snapshot accounting differs: legacy captures omit directory streams. Compare captures with the same accounting profile or rescan.".into());
                }
                let mut map = HashMap::with_capacity(old.nodes.len());
                for (id, node) in old.nodes.iter().enumerate() {
                    if progress.cancelled() {
                        return Err("Comparison cancelled.".into());
                    }
                    map.insert(comparison_key(&old, id), (node.logical, node.allocated));
                    progress.files.store(id as u64 + 1, Ordering::Relaxed);
                }
                Ok(IoResult::Baseline(Arc::new(old), map))
            });
        }
    }
    fn export_csv(&mut self, scan: &Arc<Scan>, ctx: &egui::Context) {
        if self.io_job.is_some() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("diskvale-files.csv")
            .save_file()
        {
            let scan = scan.clone();
            self.start_io(ctx, "Exporting CSV", move |progress| {
                canopy::output::csv(&scan, &path, &progress)?;
                Ok(IoResult::Exported(path))
            });
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
        if let Some(job) = &self.io_job {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
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
    matches!(
        scan.stats.runtime.provenance(),
        canopy::model::Provenance::Snapshot | canopy::model::Provenance::Replay
    )
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
        scanner::scan_drive(root, progress).map_err(|error| {
            format!(
                "Drive scan unavailable: {}. This scan did not complete.",
                error.trim_end_matches('.')
            )
        })
    }
}

fn valid_snapshot(scan: &Scan) -> bool {
    if scan.stats.schema_version > 2
        || scan.nodes.is_empty()
        || scan.nodes[0].parent.is_some()
        || !scan.nodes[0].directory
    {
        return false;
    }
    let mut seen = vec![false; scan.nodes.len()];
    let mut stack = vec![0];
    while let Some(id) = stack.pop() {
        if id >= scan.nodes.len() || seen[id] {
            return false;
        }
        seen[id] = true;
        let node = &scan.nodes[id];
        if (!node.directory && !node.children.is_empty())
            || (id != 0 && !recycle::safe_name(&node.name))
            || (node.synthetic
                && (node.directory
                    || node.reference.is_some()
                    || node.files != 0
                    || node
                        .parent
                        .is_none_or(|p| !scan.nodes.get(p).is_some_and(|n| n.directory))))
        {
            return false;
        }
        let mut names = HashSet::new();
        for &child in &scan.nodes[id].children {
            if child >= scan.nodes.len() || scan.nodes[child].parent != Some(id) {
                return false;
            }
            // NTFS can preserve case-distinct siblings. Reject exact duplicates
            // without applying a case-insensitive namespace to imported captures.
            if !scan.nodes[child].synthetic && !names.insert(scan.nodes[child].name.as_str()) {
                return false;
            }
            stack.push(child);
        }
    }
    if !seen.iter().all(|s| *s) {
        return false;
    }
    // Check derived totals rather than trusting imported aggregate/accounting claims.
    let mut totals = vec![(0u64, 0u64, 0u64, false); scan.nodes.len()];
    let mut stack = vec![(0, false)];
    while let Some((id, visited)) = stack.pop() {
        let node = &scan.nodes[id];
        if !visited && node.directory {
            stack.push((id, true));
            stack.extend(node.children.iter().map(|&c| (c, false)));
        } else if node.directory {
            let mut buckets = node.children.iter().filter(|&&c| scan.nodes[c].synthetic);
            if let Some(&bucket) = buckets.next() {
                if buckets.next().is_some()
                    || scan.nodes[bucket].logical != node.own_logical
                    || scan.nodes[bucket].allocated != node.own_allocated
                {
                    return false;
                }
            } else if node.own_logical != 0 || node.own_allocated != 0 {
                return false;
            }
            let mut sum = (0u64, 0u64, 0u64, false);
            for &child in &node.children {
                let t = totals[child];
                let (Some(a), Some(b), Some(c)) = (
                    sum.0.checked_add(t.0),
                    sum.1.checked_add(t.1),
                    sum.2.checked_add(t.2),
                ) else {
                    return false;
                };
                sum = (a, b, c, sum.3 || t.3);
            }
            totals[id] = sum;
            if (
                node.logical,
                node.allocated,
                node.files,
                node.allocation_unknown,
            ) != sum
            {
                return false;
            }
            if (node.own_logical != 0 || node.own_allocated != 0)
                && !node.children.iter().any(|&c| {
                    scan.nodes[c].synthetic
                        && scan.nodes[c].logical == node.own_logical
                        && scan.nodes[c].allocated == node.own_allocated
                })
            {
                return false;
            }
        } else {
            if node.own_logical != 0 || node.own_allocated != 0 {
                return false;
            }
            if node.files != u64::from(!node.synthetic) {
                return false;
            }
            totals[id] = (
                node.logical,
                node.allocated,
                node.files,
                node.allocation_unknown,
            );
        }
    }
    true
}
fn comparison_key(scan: &Scan, id: usize) -> String {
    if scan.nodes[id].synthetic {
        format!("\0directory-streams:{}", scan.relative_path(id))
    } else {
        scan.relative_path(id)
    }
}
fn read_snapshot(path: &std::path::Path, progress: Arc<Progress>) -> Result<Scan, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Opening snapshot: {e}"))?;
    let reader = CancellableReader {
        inner: file,
        progress: progress.clone(),
    };
    let mut scan: Scan = serde_json::from_reader(std::io::BufReader::new(reader))
        .map_err(|e| format!("Reading snapshot: {e}"))?;
    if !valid_snapshot(&scan) {
        return Err("Invalid or unsupported snapshot accounting/tree.".into());
    }
    if progress.cancelled() {
        return Err("Snapshot loading cancelled.".into());
    }
    scan.finish();
    scan.stats.files = scan.nodes[0].files;
    scan.stats.folders = scan
        .nodes
        .iter()
        .filter(|n| n.directory)
        .count()
        .saturating_sub(1) as u64;
    scan.stats.shared_links = scan
        .nodes
        .iter()
        .filter(|n| n.shared && !n.directory && !n.synthetic)
        .count() as u64;
    scan.stats.unknown_allocations = scan
        .nodes
        .iter()
        .filter(|n| n.allocation_unknown && !n.directory)
        .count() as u64;
    if !scan.stats.directory_streams_captured
        && scan
            .stats
            .filesystem
            .as_ref()
            .is_none_or(|fs| fs.eq_ignore_ascii_case("NTFS"))
    {
        scan.stats.notices.insert(0, "Legacy snapshot: directory-owned data streams were not captured; missing stream accounting cannot be reconstructed from this file. Rescan for current totals.".into());
    }
    Ok(scan)
}
fn write_csv(scan: &Scan, path: &std::path::Path) -> Result<(), String> {
    canopy::output::csv(scan, path, &Progress::default())
}
fn write_snapshot(scan: &Scan, path: &std::path::Path) -> Result<(), String> {
    canopy::output::snapshot(scan, path, &Progress::default())
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
            if let Some(path) = get("--json")
                && let Err(error) = write_snapshot(&scan, &PathBuf::from(path))
            {
                eprintln!("Snapshot output {}: {error}", path);
                std::process::exit(1);
            }
            if let Some(path) = get("--csv")
                && let Err(error) = write_csv(&scan, &PathBuf::from(path))
            {
                eprintln!("CSV output {}: {error}", path);
                std::process::exit(1);
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
    fn exfat_snapshot_round_trip_preserves_profile_without_legacy_warning_or_authority() {
        use canopy::model::{Node, Provenance, RuntimeState};
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("exfat.json");
        let mut scan = Scan {
            root: "E:\\".into(),
            nodes: vec![
                Node::new("E:\\".into(), None, true),
                Node::new("clip.mp4".into(), Some(0), false),
            ],
            stats: Default::default(),
        };
        scan.nodes[0].children = vec![1];
        scan.nodes[1].logical = 100;
        scan.nodes[1].allocated = 4096;
        scan.stats.filesystem = Some("exFAT".into());
        scan.stats.engine = "exFAT directory scan".into();
        scan.stats.schema_version = 2;
        scan.stats.runtime = RuntimeState::new(Provenance::Directory);
        scan.finish();
        write_snapshot(&scan, &path).unwrap();
        let loaded = read_snapshot(&path, Arc::default()).unwrap();
        assert_eq!(loaded.stats.filesystem.as_deref(), Some("exFAT"));
        assert_eq!(loaded.stats.files, 1);
        assert_eq!(loaded.nodes[0].allocated, 4096);
        assert!(
            loaded
                .stats
                .notices
                .iter()
                .all(|notice| !notice.contains("Legacy snapshot"))
        );
        assert_eq!(loaded.stats.runtime.provenance(), Provenance::Snapshot);
        assert!(!loaded.deletion_eligible());
    }
    #[test]
    fn review_regression_case_distinct_snapshot_round_trip_preserves_comparison_keys() {
        use canopy::model::Node;
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("case-sensitive.json");
        let mut scan = Scan {
            root: "C:\\".into(),
            stats: Default::default(),
            nodes: vec![
                Node::new("root".into(), None, true),
                Node::new("case-sensitive".into(), Some(0), true),
                Node::new("Foo.txt".into(), Some(1), false),
                Node::new("foo.txt".into(), Some(1), false),
            ],
        };
        scan.nodes[0].children = vec![1];
        scan.nodes[1].children = vec![2, 3];
        scan.nodes[2].logical = 100;
        scan.nodes[2].allocated = 4096;
        scan.nodes[3].logical = 200;
        scan.nodes[3].allocated = 8192;
        scan.stats.directory_streams_captured = true;
        scan.finish();
        write_snapshot(&scan, &path).unwrap();
        let loaded = read_snapshot(&path, Arc::default()).unwrap();
        assert_eq!(loaded.nodes[2].name, "Foo.txt");
        assert_eq!(loaded.nodes[3].name, "foo.txt");
        assert_eq!(loaded.nodes[0].allocated, 12288);
        assert_eq!(loaded.stats.files, 2);
        assert!(!loaded.deletion_eligible());
        let map: HashMap<_, _> = loaded
            .nodes
            .iter()
            .enumerate()
            .map(|(id, node)| (comparison_key(&loaded, id), (node.logical, node.allocated)))
            .collect();
        assert_eq!(map.len(), loaded.nodes.len());
        assert_eq!(map[&comparison_key(&scan, 2)], (100, 4096));
        assert_eq!(map[&comparison_key(&scan, 3)], (200, 8192));

        // Exact duplicate siblings remain malformed, even with different node IDs/sizes.
        scan.nodes[3].name = "Foo.txt".into();
        write_snapshot(&scan, &path).unwrap();
        assert!(
            read_snapshot(&path, Arc::default())
                .unwrap_err()
                .contains("Invalid")
        );
    }
    #[test]
    fn snapshot_invariants_reject_unsafe_components_overflows_and_forged_totals() {
        use canopy::model::Node;
        let mut scan = Scan {
            root: "C:\\".into(),
            stats: Default::default(),
            nodes: vec![
                Node::new("root".into(), None, true),
                Node::new("leaf.bin".into(), Some(0), false),
            ],
        };
        scan.nodes[0].children = vec![1];
        scan.nodes[1].logical = 5;
        scan.nodes[1].allocated = 4096;
        scan.finish();
        assert!(valid_snapshot(&scan));
        scan.nodes[0].allocated += 1;
        assert!(!valid_snapshot(&scan));
        scan.finish();
        scan.nodes[1].name = "../escape".into();
        assert!(!valid_snapshot(&scan));
        scan.nodes[1].name = "leaf.bin".into();
        scan.nodes[0].children.push(99);
        assert!(!valid_snapshot(&scan));
        scan.nodes[0].children.pop();
        scan.nodes
            .push(Node::new("second.bin".into(), Some(0), false));
        scan.nodes[0].children.push(2);
        scan.nodes[1].allocated = u64::MAX;
        scan.nodes[2].allocated = 1;
        scan.finish();
        assert!(!valid_snapshot(&scan));
    }
    #[test]
    fn upgrading_legacy_snapshot_preserves_missing_accounting_and_recomputes_counts() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("legacy.json");
        use canopy::model::Node;
        let scan = Scan {
            root: "C:\\".into(),
            stats: Default::default(),
            nodes: vec![Node::new("root".into(), None, true)],
        };
        canopy::output::snapshot(&scan, &path, &Progress::default()).unwrap();
        let loaded = read_snapshot(&path, Arc::default()).unwrap();
        assert_eq!(loaded.stats.schema_version, 2);
        assert!(!loaded.stats.directory_streams_captured);
        assert!(
            loaded
                .stats
                .notices
                .iter()
                .any(|n| n.contains("cannot be reconstructed"))
        );
        assert!(!loaded.deletion_eligible());
        assert_eq!(loaded.stats.files, 0);
    }
    #[test]
    fn regression_import_requires_directory_root_and_leaf_files() {
        use canopy::model::Node;
        let mut scan = Scan {
            root: "C:\\".into(),
            stats: Default::default(),
            nodes: vec![Node::new("root".into(), None, false)],
        };
        assert!(!valid_snapshot(&scan));
        scan.nodes[0].directory = true;
        scan.nodes.push(Node::new("file".into(), Some(0), false));
        scan.nodes.push(Node::new("child".into(), Some(1), false));
        scan.nodes[0].children = vec![1];
        scan.nodes[1].children = vec![2];
        assert!(!valid_snapshot(&scan));
    }
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
        assert!(error.contains("requires a whole local drive"), "{error}");
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
