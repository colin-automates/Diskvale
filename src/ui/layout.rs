use super::*;
use egui::{Align2, Sense};

impl Canopy {
    #[cfg(windows)]
    fn drive_choices(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.drive_receiver.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Finding connected drives…");
            });
        }
        for drive in &self.drives {
            let supported = scanner::DriveScanMethod::for_filesystem(&drive.filesystem).is_some();
            let response = ui.add_enabled(
                supported && self.receiver.is_none(),
                egui::Button::new(format!("{} {}", drive.root.display(), drive.filesystem)),
            );
            if response
                .on_hover_text(if supported {
                    "Choose this drive for the next scan."
                } else {
                    "Supported filesystems: NTFS, exFAT and FAT32."
                })
                .clicked()
            {
                self.path = drive.root.display().to_string();
                self.message = format!("Next scan target: {} ({})", self.path, drive.filesystem);
                ui.close();
            }
        }
        ui.separator();
        if ui
            .add_enabled(
                self.drive_receiver.is_none(),
                egui::Button::new("Refresh drives"),
            )
            .clicked()
        {
            self.refresh_drives(ctx.clone());
        }
    }

    pub(super) fn size(&self, value: u64) -> String {
        self.units.format(value)
    }

    pub(super) fn select(&mut self, id: usize) {
        if self.scan.as_ref().is_none_or(|scan| id >= scan.nodes.len()) {
            return;
        }
        self.selected = Some(id);
        self.sync_selection();
    }

    fn sync_selection(&mut self) {
        if self.selected == self.tree_selection {
            return;
        }
        self.tree_selection = self.selected;
        if let (Some(scan), Some(id)) = (&self.scan, self.selected) {
            let Some(node) = scan.nodes.get(id) else {
                self.selected = None;
                self.tree_selection = None;
                return;
            };
            let mut parent = node.parent;
            while let Some(id) = parent {
                self.expanded.insert(id);
                parent = scan.nodes[id].parent;
            }
            self.tree_dirty = true;
            self.scroll_selection = true;
        }
    }

    fn invalidate_metric(&mut self) {
        self.dirty = true;
        self.map_key = None;
        self.types_key = None;
    }

    fn filters(&mut self, ctx: &egui::Context) {
        let key = (self.metric, self.query.clone(), self.category);
        if let Some(receiver) = &self.filter_receiver {
            match receiver.try_recv() {
                Ok(totals) => {
                    if self.filtering_key.as_ref() == Some(&key) {
                        self.weights = Some(totals.bytes);
                        self.matching_files = Some(totals.files);
                        self.filter_key = Some(key.clone());
                        self.map_key = None;
                    }
                    self.filter_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.filter_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.query.is_empty() && self.category.is_none() {
            self.matching_files = None;
            if self.weights.take().is_some() {
                self.map_key = None;
            }
            self.filter_key = Some(key);
        } else if self.filter_key.as_ref() != Some(&key)
            && self.filter_receiver.is_none()
            && let Some(scan) = self.scan.clone()
        {
            let predicate = Query::parse(&self.query);
            let metric = self.metric;
            let category = self.category;
            let (sender, receiver) = mpsc::channel();
            self.filtering_key = Some(key);
            self.filter_receiver = Some(receiver);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = sender.send(query::map_totals(&scan, metric, &predicate, category));
                ctx.request_repaint();
            });
        }
        if self.filter_receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn search_field(&mut self, ui: &mut egui::Ui, width: f32) {
        ui.push_id("search-field", |ui| {
            let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
            ui.painter().rect_filled(rect, 4, BG);
            let text_rect =
                Rect::from_min_max(rect.min + vec2(10.0, 0.0), rect.max - vec2(64.0, 0.0));
            let edit = ui.put(
                text_rect,
                egui::TextEdit::singleline(&mut self.query)
                    .id(egui::Id::new("global-search"))
                    .hint_text("Search files…")
                    .font(FontId::proportional(14.0))
                    .vertical_align(egui::Align::Center)
                    .margin(0)
                    .frame(false),
            );
            if self.search_focus || response.clicked() {
                edit.request_focus();
                self.search_focus = false;
            }
            if edit.changed() {
                self.dirty = true;
            }
            if self.query.is_empty() {
                let shortcut = Rect::from_center_size(
                    egui::pos2(rect.right() - 30.0, rect.center().y),
                    vec2(44.0, 20.0),
                );
                ui.painter().rect_filled(shortcut, 3, RAISED);
                if ui
                    .put(
                        shortcut,
                        egui::Label::new(RichText::new("Ctrl+K").size(11.0).color(MUTED))
                            .sense(Sense::click()),
                    )
                    .clicked()
                {
                    edit.request_focus();
                }
            } else {
                let clear = Rect::from_center_size(
                    egui::pos2(rect.right() - 20.0, rect.center().y),
                    vec2(24.0, 24.0),
                );
                if ui
                    .put(
                        clear,
                        egui::Button::new(RichText::new("×").size(17.0)).frame(false),
                    )
                    .on_hover_text("Clear search")
                    .clicked()
                {
                    self.query.clear();
                    self.dirty = true;
                    edit.request_focus();
                }
            }
            let stroke = if edit.has_focus() {
                ui.visuals().selection.stroke
            } else {
                Stroke::new(1.0_f32, if response.hovered() { MUTED } else { BORDER })
            };
            ui.painter()
                .rect_stroke(rect, 4, stroke, egui::StrokeKind::Inside);
            edit.on_hover_ui(|ui| {
                ui.set_max_width(420.0);
                ui.label(RichText::new("Search files").strong());
                ui.label("Search names or combine these filters:");
                egui::Grid::new("search-help")
                    .spacing(vec2(16.0, 6.0))
                    .show(ui, |ui| {
                        for (syntax, meaning) in [
                            ("ext:mp4", "File type"),
                            (">1GB  <500MB", "Size in the selected measure"),
                            ("older:1y", "Not modified for a year"),
                            ("newer:30d", "Modified in the last 30 days"),
                        ] {
                            ui.monospace(syntax);
                            ui.label(meaning);
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.label("Age units: d, w, m, y. GB is decimal; GiB is binary.");
                ui.label(
                    "Search applies to Map, Files and Types, and stays on when you open a folder.",
                );
            });
        });
    }

    fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("brand").exact_height(52.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(14, 9)))
            .show(ctx, |ui| { ui.horizontal(|ui| {
                brand::header_mark(ui);
                ui.label(RichText::new(brand::NAME).size(22.0).strong());
                ui.add_space(6.0);
                for (view,title) in [(View::Folders,"Map"),(View::Largest,"Files"),(View::Types,"Types")] {
                    if ui.selectable_label(self.view==view,title).clicked() { self.view=view; self.dirty=true; }
                }
                ui.add_space(8.0);
                let width = (ui.available_width()-320.0).max(180.0);
                self.search_field(ui, width);
                if self.receiver.is_some() {
                    let stopping=self.progress.cancelled();
                    if ui.add_enabled(!stopping,egui::Button::new(if stopping {"Stopping…"} else {"Cancel"})).clicked() { self.progress.cancel.store(true,Ordering::Relaxed); }
                } else {
                    let first=self.scan.is_none();
                    let button=egui::Button::new(if first {RichText::new("Scan drive").color(BG).strong()} else {RichText::new("Rescan")});
                    let button=if first {button.fill(ACCENT)} else {button};
                    if ui.add(button).on_hover_text(format!("Scan {}. NTFS uses its index; exFAT/FAT32 scans folder metadata.",self.path)).clicked() { self.start_scan(ctx.clone()); }
                }
                ui.menu_button("Open / save",|ui| {
                    if ui.add_enabled(self.receiver.is_none(),egui::Button::new("Open saved scan…")).clicked() {
                        ui.close();
                        if let Some(path)=rfd::FileDialog::new().add_filter("Diskvale snapshot", &["json"]).pick_file() { self.load_scan(ctx.clone(),path); }
                    }
                    if let Some(scan)=self.scan.clone() {
                        if ui.add_enabled(self.io_job.is_none(), egui::Button::new("Save snapshot…")).clicked() { ui.close(); self.save_snapshot(&scan, ctx); }
                        if ui.add_enabled(self.io_job.is_none(), egui::Button::new("Export CSV…")).clicked() { ui.close(); self.export_csv(&scan, ctx); }
                        if ui.add_enabled(self.io_job.is_none(), egui::Button::new("Compare snapshot…")).clicked() { ui.close(); self.load_baseline(&scan, ctx); }
                    }
                    ui.separator();
                    if ui.button("Compact window").clicked() {ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(1050.0,700.0)));ui.close();}
                    if ui.button("Roomy window").clicked() {ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(1400.0,900.0)));ui.close();}
                });
                egui::Frame::new().stroke(Stroke::new(1.0_f32,BORDER)).corner_radius(6).inner_margin(5).show(ui,|ui| {
                    ui.label(RichText::new("Recycle Bin").size(12.0));
                }).response.on_hover_text("Delete actions use the Recycle Bin. If Windows cannot recycle an item, permanent deletion requires a separate confirmation.");
            }); });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let compact = ctx.content_rect().width() < 1200.0;
        egui::SidePanel::left("sidebar").default_width(if compact {238.0} else {280.0}).width_range(200.0..=440.0).resizable(true)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(12))
            .show(ctx,|ui| {
                ui.label(RichText::new("STORAGE").size(12.0).color(MUTED));
                let scan=self.scan.clone();
                let root=scan.as_ref().map_or(self.path.clone(),|s|s.root.display().to_string());
                #[cfg(windows)]
                let live=self.drives.iter().find(|d|root.eq_ignore_ascii_case(&d.root.display().to_string())).cloned();
                #[cfg(not(windows))]
                let live: Option<&()> = None;
                let captured=scan.as_ref().and_then(|s|s.stats.volume_space.as_ref());
                #[cfg(windows)]
                let space=captured.map(|s|(s.capacity,s.free)).or_else(||live.as_ref().map(|d|(d.total,d.free)));
                #[cfg(not(windows))]
                let space=captured.map(|s|(s.capacity,s.free));
                egui::Frame::new().fill(RAISED).stroke(Stroke::new(1.0_f32,BORDER)).corner_radius(7).inner_margin(10).show(ui,|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.menu_button(RichText::new(&root).size(21.0).strong(),|ui| {
                        #[cfg(windows)]
                        self.drive_choices(ui, ctx);
                    }).response.on_hover_text("Choose the drive for the next scan. Current results keep their captured root.");
                        #[cfg(windows)]
                        let filesystem = scan.as_ref().and_then(|s| s.stats.filesystem.as_deref()).or_else(||live.as_ref().map(|d|d.filesystem.as_str())).unwrap_or("Drive");
                        #[cfg(not(windows))]
                        let filesystem = scan.as_ref().and_then(|s| s.stats.filesystem.as_deref()).unwrap_or("Drive");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {ui.label(RichText::new(filesystem).size(12.0).color(MUTED));});
                    });
                    if let Some((total,free))=space {
                        let used=total.saturating_sub(free); let fraction=free as f32/total.max(1) as f32;
                        let tint=if fraction<0.10 {AMBER} else {Color32::from_rgb(104,147,174)};
                        ui.label(format!("{} capacity",self.size(total)));
                        ui.label(RichText::new(format!("{} free · {:.1}%",self.size(free),fraction*100.0)).color(tint));
                        ui.add(egui::ProgressBar::new(used as f32/total.max(1) as f32).fill(tint).desired_height(6.0));
                        ui.label(RichText::new(format!("{} used",self.size(used))).color(MUTED));
                        if scan.is_some() {
                            ui.label(RichText::new(if captured.is_some() {"Storage at scan time"} else {"Live storage · saved index"}).size(12.0).color(MUTED))
                                .on_hover_text("A saved index without captured volume space cannot establish an unaccounted-space remainder.");
                        }
                    } else { ui.label(RichText::new("Capacity unavailable").color(MUTED)); }
                    if let Some(scan)=&scan {ui.label(RichText::new(format!("{} files · {} folders",count(scan.stats.files),count(scan.stats.folders))).size(12.0).color(MUTED));}
                });
                if self.scan.is_none() {
                    ui.add_space(10.0);
                    #[cfg(windows)]
                    ui.menu_button("Choose drive",|ui| {
                        self.drive_choices(ui, ctx);
                    });
                    let _ = live;
                    return;
                }
                let scan=scan.unwrap();
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("CONTENTS").size(12.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                        ui.menu_button("Options", |ui| {
                            if ui.checkbox(&mut self.show_zero_entries, "Show zero-size entries").changed() {
                                self.tree_dirty = true;
                                self.scroll_selection = true;
                            }
                            ui.label(RichText::new("Entries with 0 B in the current size metric are hidden by default.").size(12.0).color(MUTED));
                            ui.separator();
                            if egui::ComboBox::from_id_salt("units").selected_text(format!("Units: {}",self.units.label())).width(83.0).show_ui(ui,|ui| {
                                let old=self.units;ui.selectable_value(&mut self.units,Units::Decimal,"GB · decimal");ui.selectable_value(&mut self.units,Units::Binary,"GiB · binary");old!=self.units
                            }).inner.unwrap_or(false) {self.map_key=None;}
                        });
                    });
                });
                let scope = &scan.nodes[self.current];
                egui::Frame::new().fill(RAISED).corner_radius(4).inner_margin(6).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(parent) = scope.parent
                            && ui.small_button("‹").on_hover_text(format!("Parent folder: {}", scan.path(parent).display())).clicked() {
                                self.navigate(parent);
                        }
                        let available = ui.available_width();
                        ui.allocate_ui_with_layout(vec2(available, 20.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(self.size(scope.size(self.metric))).size(12.0).monospace().color(MUTED));
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.add(egui::Label::new(RichText::new(&scope.name).strong()).truncate())
                                    .on_hover_text(scan.path(self.current).display().to_string());
                            });
                        });
                    });
                });
                ui.add_space(4.0);
                self.sync_selection();
                if self.tree_dirty {
                    self.tree_dirty=false; self.tree_rows.clear();
                    // Bound huge directory fan-out, but always retain selected ancestry.
                    let selected_path=ancestor_set(&scan,self.selected.unwrap_or(self.current));
                    let mut stack=vec![TreeRow::Entry{id:self.current,depth:0}];
                    while let Some(row)=stack.pop() {
                        if !matches!(row,TreeRow::Entry{id,..} if id==self.current) {self.tree_rows.push(row);}
                        let TreeRow::Entry{id,depth}=row else {continue};
                        if id!=self.current && !self.expanded.contains(&id) {continue;}
                        let mut children:Vec<_>=scan.nodes[id].children.iter().copied()
                            .filter(|child|self.show_zero_entries || scan.nodes[*child].size(self.metric)>0 || selected_path.contains(child)).collect();
                        children.sort_unstable_by_key(|&id|std::cmp::Reverse(scan.nodes[id].size(self.metric)));
                        let total=children.len();
                        let page=*self.tree_pages.get(&id).unwrap_or(&0);
                        let start=page.saturating_mul(TREE_FANOUT).min(total.saturating_sub(1));
                        let mut visible:Vec<_>=children.iter().skip(start).take(TREE_FANOUT).copied().collect();
                        for child in children.iter().copied().filter(|child|selected_path.contains(child)) { if !visible.contains(&child) { visible.push(child); } }
                        // Pushed first so it pops last, after every visible child and its subtree.
                        if total>visible.len() { stack.push(TreeRow::More{parent:id,depth:depth+1,hidden:total-visible.len(),next_page:if (page+1)*TREE_FANOUT>=total {0} else {page+1}}); }
                        stack.extend(visible.into_iter().rev().map(|child|TreeRow::Entry{id:child,depth:depth+1}));
                    }
                }
                let row_height=28.0;
                let selected_index=self.tree_rows.iter().position(|row|matches!(row,TreeRow::Entry{id,..} if Some(*id)==self.selected));
                let mut scroll=egui::ScrollArea::vertical().id_salt(("folder-tree",self.current)).auto_shrink([false,false]);
                if self.scroll_selection && let Some(index)=selected_index {
                    scroll=scroll.vertical_scroll_offset((index as f32*row_height-ui.available_height()*0.4).max(0.0));
                } else if self.scroll_selection {
                    scroll=scroll.vertical_scroll_offset(0.0);
                }
                self.scroll_selection=false;
                let rows=self.tree_rows.clone();
                if rows.is_empty() {
                    ui.label(RichText::new(if scope.children.is_empty() {"This folder is empty."} else {"No entries with nonzero size."}).size(12.0).color(MUTED));
                }
                scroll.show_rows(ui,row_height,rows.len(),|ui,range| {
                    for index in range {
                        let (id,depth)=match rows[index] {
                            TreeRow::Entry{id,depth}=>(id,depth),
                            TreeRow::More{parent,depth,hidden,next_page}=>{
                                let (rect,response)=ui.allocate_exact_size(vec2(ui.available_width(),row_height-2.0),Sense::click());
                                if response.hovered() {ui.painter().rect_filled(rect,3,RAISED);}
                                ui.painter().text(egui::pos2(rect.left()+21.0+tree_indent(depth.saturating_sub(1)),rect.center().y),Align2::LEFT_CENTER,format!("… {} more",count(hidden as u64)),FontId::proportional(12.0),MUTED);
                                if response.clicked() {
                                    self.tree_pages.insert(parent,next_page);
                                    self.tree_dirty=true;
                                }
                                response.on_hover_text(format!("Show the next page of up to {TREE_FANOUT} direct children, including empty folders. The last page cycles to the first."));
                                continue;
                            }
                        };
                        let node=&scan.nodes[id];
                        let (rect,response)=ui.allocate_exact_size(vec2(ui.available_width(),row_height-2.0),Sense::click());
                        let share=node.size(self.metric) as f32/node.parent.map_or(node.size(self.metric).max(1),|p|scan.nodes[p].size(self.metric).max(1)) as f32;
                        if response.hovered() || self.selected==Some(id) {ui.painter().rect_filled(rect,3,RAISED);}
                        let bar=Rect::from_min_size(egui::pos2(rect.right()-74.0,rect.bottom()-4.0),vec2(70.0*share.clamp(0.0,1.0),3.0));
                        // Files wear their map colour, so the tree reads like the map.
                        ui.painter().rect_filled(bar,0,if node.directory {Color32::from_rgb(64,83,97)} else {file_color(&node.name)});
                        if self.selected==Some(id) {ui.painter().rect_stroke(rect,3,Stroke::new(1.0_f32,ACCENT),egui::StrokeKind::Inside);}
                        let x=rect.left()+4.0+tree_indent(depth.saturating_sub(1));
                        let chevron=Rect::from_min_size(egui::pos2(x,rect.top()),vec2(14.0,rect.height()));
                        if node.directory && !node.children.is_empty() {
                            let c=chevron.center(); let points=if self.expanded.contains(&id){vec![c+vec2(-4.0,-2.0),c+vec2(4.0,-2.0),c+vec2(0.0,3.0)]}else{vec![c+vec2(-2.0,-4.0),c+vec2(-2.0,4.0),c+vec2(3.0,0.0)]}; ui.painter().add(egui::Shape::convex_polygon(points,MUTED,Stroke::NONE));
                            if ui.interact(chevron,ui.id().with(("expand",id)),Sense::click()).clicked() {
                                if !self.expanded.remove(&id) {self.expanded.insert(id);} self.tree_dirty=true;
                            }
                        }
                        let title=if id==0 {scan.root.display().to_string()} else {node.name.clone()};
                        let mut job=egui::text::LayoutJob::simple(title,FontId::proportional(13.0),if node.directory {TEXT} else {MUTED},(rect.width()-(x-rect.left())-105.0).max(16.0));
                        job.wrap.max_rows=1;
                        ui.painter().galley(egui::pos2(x+17.0,rect.top()+5.0),ui.painter().layout_job(job),TEXT);
                        ui.painter().text(egui::pos2(rect.right()-5.0,rect.center().y),Align2::RIGHT_CENTER,self.size(node.size(self.metric)),FontId::monospace(12.0),MUTED);
                        if response.double_clicked() && node.directory {self.navigate(id);}
                        else if response.clicked() && response.interact_pointer_pos().is_some_and(|p|p.x>chevron.right()) {self.select(id);}
                        response.on_hover_text(format!("{}\n{} · {:.2}% of parent",scan.path(id).display(),self.size(node.size(self.metric)),share*100.0));
                    }
                });
            });
    }

    fn actions(&mut self, ui: &mut egui::Ui, scan: &Arc<Scan>) {
        // Navigation gets the full width. Whole ancestors collapse into a menu
        // instead of a scroller starting halfway through a folder name.
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.current != 0,
                    egui::Button::new("Up").min_size(vec2(0.0, 28.0)),
                )
                .on_hover_text("Parent folder · Backspace")
                .clicked()
            {
                self.navigate(scan.nodes[self.current].parent.unwrap_or(0));
            }
            self.breadcrumbs(ui, scan);
        });
        ui.horizontal_wrapped(|ui| {
            let node = &scan.nodes[self.current];
            let folder = self.current != 0;
            let (allocated_title, logical_title) = if folder {("Folder size on disk", "Folder file size")} else {("Size on disk", "File size")};
            let title = match self.metric { Metric::Allocated => allocated_title, Metric::Logical => logical_title };
            let previous = self.metric;
            egui::ComboBox::from_id_salt("area-metric")
                .selected_text(format!("{title} · {}", self.size(node.size(self.metric))))
                .width(if folder {260.0} else {220.0})
                .show_ui(ui, |ui| {
                    for (metric, title, size) in [(Metric::Allocated, allocated_title, node.allocated), (Metric::Logical, logical_title, node.logical)] {
                        ui.selectable_value(&mut self.metric, metric, format!("{title} · {}", self.units.format(size)));
                    }
                }).response.on_hover_text(if folder {
                    "Totals for files inside this folder, including subfolders. Folder size on disk counts physical space once for shared hard links. Folder file size sums the bytes a program would read at every path. Area follows the selected measure; sparse, compressed, resident and cloud files can differ between the two."
                } else {
                    "Area follows this size measure. Size on disk counts physical space once for shared hard links. File size counts the bytes a program would read at every path. Sparse, compressed, resident and cloud files can differ between the two."
                });
            if previous != self.metric { self.invalidate_metric(); self.tree_dirty = true; }
            if self.view == View::Folders {
                self.map_options(ui);
            } else {
                let dated=self.by_modified && self.view!=View::Types;
                if ui.small_button(match (dated,self.descending) {(true,true)=>"Newest first",(true,false)=>"Oldest first",(false,true)=>"Largest first",(false,false)=>"Smallest first"}).clicked() {self.descending = !self.descending;self.dirty=true;}
            }
        });
    }

    fn map_options(&mut self, ui: &mut egui::Ui) {
        let old = (self.depth, self.minimum);
        let custom = old != (96, 0);
        let title = if custom {
            "Map options · Custom"
        } else {
            "Map options"
        };
        let response = ui.button(title);
        egui::Popup::menu(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(300.0);
                ui.label(RichText::new("Detail level").size(12.0).color(MUTED));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.depth, 96, "All");
                    for depth in 1..=4 {
                        ui.selectable_value(&mut self.depth, depth, depth.to_string())
                            .on_hover_text(format!(
                                "{depth} level{}",
                                if depth == 1 { "" } else { "s" }
                            ));
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Group below").size(12.0).color(MUTED));
                ui.horizontal(|ui| {
                    let megabyte = match self.units {
                        Units::Decimal => 1_000_000,
                        Units::Binary => 1_048_576,
                    };
                    for multiple in [0, 1, 10, 100] {
                        let size = multiple * megabyte;
                        let label = if size == 0 {
                            "None".into()
                        } else {
                            self.size(size)
                        };
                        ui.selectable_value(&mut self.minimum, size, label);
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Grouped regions keep every byte in the map.")
                        .size(12.0)
                        .color(MUTED),
                );
                ui.separator();
                if ui
                    .add_enabled(custom, egui::Button::new("Reset map options"))
                    .clicked()
                {
                    self.depth = 96;
                    self.minimum = 0;
                    ui.close();
                }
            });
        response.on_hover_text(format!(
            "Detail level: {}\nGroup below: {}",
            if self.depth == 96 {
                "All descendants".into()
            } else {
                self.depth.to_string()
            },
            if self.minimum == 0 {
                "No grouping".into()
            } else {
                self.size(self.minimum)
            }
        ));
        if old != (self.depth, self.minimum) {
            self.map_key = None;
        }
    }

    fn breadcrumbs(&mut self, ui: &mut egui::Ui, scan: &Scan) {
        let mut crumbs = vec![];
        let mut id = Some(self.current);
        while let Some(index) = id {
            let title = if index == 0 {
                scan.root.display().to_string()
            } else {
                scan.nodes[index].name.clone()
            };
            crumbs.push((index, title));
            id = scan.nodes[index].parent;
        }
        crumbs.reverse();
        let font = egui::TextStyle::Button.resolve(ui.style());
        let text_width = |text: &str| {
            ui.painter()
                .layout_no_wrap(text.into(), font.clone(), TEXT)
                .size()
                .x
        };
        let padding = ui.spacing().button_padding.x * 2.0;
        let separator = text_width("›") + ui.spacing().item_spacing.x * 2.0;
        let overflow = text_width("…") + padding + separator;
        let mut hidden = vec![];
        while crumbs.len() > 2 {
            let width: f32 = crumbs
                .iter()
                .map(|(_, title)| text_width(title) + padding)
                .sum::<f32>()
                + separator * (crumbs.len() - 1) as f32
                + if hidden.is_empty() { 0.0 } else { overflow };
            if width <= ui.available_width() {
                break;
            }
            hidden.push(crumbs.remove(1));
        }
        for (position, (id, title)) in crumbs.into_iter().enumerate() {
            if position != 0 {
                ui.label(RichText::new("›").color(MUTED));
            }
            if position == 1 && !hidden.is_empty() {
                ui.menu_button("…", |ui| {
                    ui.set_max_width(480.0_f32.min(ui.ctx().content_rect().width() - 24.0));
                    for (id, title) in &hidden {
                        if ui
                            .add(egui::Button::new(title).truncate())
                            .on_hover_text(scan.path(*id).display().to_string())
                            .clicked()
                        {
                            self.navigate(*id);
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Earlier folders in this path");
                ui.label(RichText::new("›").color(MUTED));
            }
            if ui
                .add(
                    egui::Button::selectable(id == self.current, title)
                        .truncate()
                        .min_size(vec2(0.0, 28.0)),
                )
                .on_hover_text(scan.path(id).display().to_string())
                .clicked()
            {
                self.navigate(id);
            }
        }
    }

    fn map(&mut self, ui: &mut egui::Ui, scan: &Arc<Scan>, height: f32) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), height.max(60.0)), Sense::click());
        let key = (
            self.current,
            self.metric,
            rect.width() as i32,
            rect.height() as i32,
            rect.min.x as i32,
            rect.min.y as i32,
            (ui.ctx().pixels_per_point() * 100.0) as i32,
        );
        let filtered = !self.query.is_empty() || self.category.is_some();
        let outside = if self.current == 0 && self.metric == Metric::Allocated && !filtered {
            scan.outside_indexed().unwrap_or(0)
        } else {
            0
        };
        let mut remainder = None;
        let map_rect = if outside > 0 {
            let regions = treemap::layout(
                &[(0, scan.nodes[0].allocated), (1, outside)],
                treemap::Rect {
                    x: rect.left(),
                    y: rect.top(),
                    w: rect.width(),
                    h: rect.height(),
                },
            );
            let r = regions.iter().find(|(id, _)| *id == 1).unwrap().1;
            remainder = Some(Rect::from_min_size(egui::pos2(r.x, r.y), vec2(r.w, r.h)));
            regions
                .iter()
                .find(|(id, _)| *id == 0)
                .map(|(_, r)| *r)
                .unwrap_or(treemap::Rect {
                    x: rect.left(),
                    y: rect.top(),
                    w: 0.0,
                    h: 0.0,
                })
        } else {
            treemap::Rect {
                x: rect.left(),
                y: rect.top(),
                w: rect.width(),
                h: rect.height(),
            }
        };
        if self.map_key != Some(key) {
            self.map_cache = treemap::with_options(
                scan,
                self.current,
                self.metric,
                map_rect,
                treemap::Options {
                    weights: if filtered {
                        self.weights.as_deref()
                    } else {
                        None
                    },
                    counts: if filtered {
                        self.matching_files.as_deref()
                    } else {
                        None
                    },
                    depth: self.depth,
                    minimum: self.minimum,
                },
            );
            let (mesh, labels) = self.map_artwork(ui, scan, filtered);
            self.map_shapes = vec![egui::Shape::mesh(mesh)];
            self.map_shapes.extend(labels);
            self.map_key = Some(key);
        }
        ui.painter().rect_filled(rect, 0, PANEL);
        ui.painter().extend(self.map_shapes.iter().cloned());
        if let Some(block) = remainder {
            ui.painter()
                .rect_filled(block, 0, Color32::from_rgb(79, 86, 94));
            // A thin strip cannot hold readable text; its tooltip carries the detail.
            if block.width() >= 80.0 && block.height() >= 36.0 {
                let mut job = egui::text::LayoutJob::simple(
                    format!("Outside indexed files\n{}", self.size(outside)),
                    FontId::proportional(12.0),
                    TEXT,
                    (block.width() - 8.0).max(1.0),
                );
                job.wrap.max_rows = 4;
                ui.painter().galley(
                    block.min + vec2(4.0, 4.0),
                    ui.painter().layout_job(job),
                    TEXT,
                );
            }
        }
        if self.map_cache.is_empty() {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                if filtered {
                    "No files match this filter"
                } else {
                    "No occupied space in this folder"
                },
                FontId::proportional(16.0),
                MUTED,
            );
        }
        if let Some(id) = self.selected {
            for tile in self
                .map_cache
                .iter()
                .filter(|t| t.id == id && t.kind != treemap::Kind::Aggregate)
            {
                let block = tile_rect(tile);
                ui.painter().rect_stroke(
                    block,
                    0,
                    Stroke::new(2.0_f32, ACCENT),
                    egui::StrokeKind::Inside,
                );
            }
        }
        let under_pointer = map_pointer(&response)
            .and_then(|pos| {
                self.map_cache
                    .iter()
                    .rev()
                    .find(|tile| tile_rect(tile).contains(pos))
            })
            .copied();
        // While the right-click menu is open the map stops reacting, so the
        // tooltip cannot cover the menu the pointer is heading for.
        let menu_open = response.context_menu_opened();
        let hovered = under_pointer.filter(|_| !menu_open);
        if response.secondary_clicked() {
            self.map_menu_is_group =
                under_pointer.is_some_and(|tile| tile.kind == treemap::Kind::Aggregate);
            self.map_menu = under_pointer.map(|tile| tile.id);
            if let Some(id) = self.map_menu {
                self.select(id);
            }
        }
        if self.map_menu.is_some() || menu_open {
            response.context_menu(|ui| match self.map_menu {
                Some(id) => self.map_menu_items(ui, scan, id),
                // Right-clicking empty space opens nothing.
                None => ui.close(),
            });
        }
        if let Some(tile) = hovered {
            let node = &scan.nodes[tile.id];
            ui.painter().rect_stroke(
                tile_rect(&tile),
                0,
                Stroke::new(1.0_f32, TEXT),
                egui::StrokeKind::Inside,
            );
            if response.double_clicked() {
                let folder = if node.directory {
                    tile.id
                } else {
                    node.parent.unwrap_or(0)
                };
                self.navigate(folder);
                if !node.directory {
                    self.select(tile.id);
                }
            } else if response.clicked() {
                self.select(tile.id);
            }
            // Map inspection follows the pointer immediately, including while it
            // moves between blocks. Other controls keep the standard hover delay.
            let grouped = tile.kind == treemap::Kind::Aggregate;
            instant_map_tooltip(&response, (tile.id, grouped), |ui| {
                let folder = if grouped {
                    scan.path(tile.id)
                } else {
                    node.parent
                        .map_or_else(|| scan.root.clone(), |id| scan.path(id))
                };
                ui.label(
                    RichText::new(if grouped { "Grouped files" } else { &node.name })
                        .size(15.0)
                        .strong(),
                );
                let (measure, size) = if grouped {
                    (
                        if self.metric == Metric::Allocated {
                            "Size on disk"
                        } else {
                            "File size"
                        },
                        tile.size,
                    )
                } else {
                    ("File size", node.logical)
                };
                ui.label(RichText::new(format!("{measure}: {}", self.size(size))).strong());
                ui.label(format!("Location: {}", folder.display()));
            });
        } else if !menu_open
            && map_pointer(&response).is_some_and(|pos| remainder.is_some_and(|r| r.contains(pos)))
        {
            instant_map_tooltip(&response, "outside", |ui| {
                ui.label(RichText::new("Outside indexed files").size(15.0).strong());
                ui.label(RichText::new(format!("Size on disk: {}", self.size(outside))).strong());
                ui.label(format!("Location: {}", scan.root.display()));
            });
        }
    }

    /// Right-click actions for a map entry; `id` is a file or a folder.
    fn map_menu_items(&mut self, ui: &mut egui::Ui, scan: &Arc<Scan>, id: usize) {
        let node = &scan.nodes[id];
        ui.label(RichText::new(&node.name).strong());
        ui.separator();
        if node.directory {
            if id != self.current && ui.button("Zoom in").clicked() {
                self.navigate(id);
                ui.close();
            }
        } else {
            let category = Category::from_name(&node.name);
            if ui
                .button(format!("Only show {}", category.label()))
                .on_hover_text("Isolate this file type in every view")
                .clicked()
            {
                self.category = Some(category);
                self.dirty = true;
                ui.close();
            }
        }
        if ui.button("Show in Explorer").clicked() {
            reveal(&scan.path(id));
            ui.close();
        }
        if ui.button("Copy path").clicked() {
            ui.ctx().copy_text(scan.path(id).display().to_string());
            ui.close();
        }
        self.delete_menu_item(ui, scan, id, self.map_menu_is_group);
    }

    pub(super) fn delete_menu_item(
        &mut self,
        ui: &mut egui::Ui,
        scan: &Scan,
        id: usize,
        grouped: bool,
    ) {
        ui.separator();
        let enabled = id != 0
            && !grouped
            && !self.scan_stale
            && self.receiver.is_none()
            && self.deletion.is_none()
            && self.delete_pending.is_none();
        let enabled = enabled
            && scan.deletion_eligible()
            && self
                .scan
                .as_ref()
                .is_some_and(|s| std::ptr::eq(s.as_ref(), scan))
            && scan
                .nodes
                .get(id)
                .is_some_and(|n| !n.synthetic && n.reference.is_some() && !n.reparse);
        let response = ui.add_enabled(
            enabled,
            egui::Button::new(RichText::new("Delete…").color(AMBER)),
        );
        let response = if id == 0 {
            response.on_disabled_hover_text("Drive and scan roots cannot be deleted")
        } else if grouped {
            response.on_disabled_hover_text("Zoom in to choose an individual file or folder")
        } else if !scan.deletion_eligible() || self.scan_stale {
            response.on_disabled_hover_text(
                "Requires a fresh complete live scan with verified NTFS identities. Rescan the drive.",
            )
        } else {
            response.on_hover_text("Confirm the exact path, then move it to the Recycle Bin")
        };
        if response.clicked() {
            match recycle::Target::from_scan(scan, id) {
                Ok(target) => {
                    self.delete_pending =
                        Some(DeleteConfirmation::new(target, recycle::Method::RecycleBin));
                    self.delete_pending.as_mut().unwrap().generation = self.generation;
                }
                Err(error) => self.scan_error = Some(error),
            }
            ui.close();
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.deletion {
            egui::Modal::new(egui::Id::new("recycle-progress")).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.heading(match job.method {
                    recycle::Method::RecycleBin => "Moving to Recycle Bin…",
                    recycle::Method::Permanent => "Permanently deleting…",
                });
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Waiting for Windows to finish this item.");
                });
            });
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let Some(confirmation) = self.delete_pending.clone() else {
            return;
        };
        if confirmation.generation != self.generation {
            self.delete_pending = None;
            return;
        }
        let target = &confirmation.target;
        let permanent = confirmation.method == recycle::Method::Permanent;
        // Distinct modal IDs keep the first dialog's focus out of the second.
        let modal = egui::Modal::new(egui::Id::new(("confirm-delete", confirmation.method))).show(ctx, |ui| {
            ui.set_width(560.0_f32.min(ctx.content_rect().width() - 48.0));
            ui.heading(match (permanent, target.directory) {
                (true, true) => "Permanently delete this folder?",
                (true, false) => "Permanently delete this file?",
                (false, true) => "Delete this folder?",
                (false, false) => "Delete this file?",
            });
            if permanent { ui.label("Windows cannot recycle this item."); ui.add_space(6.0); }
            ui.label(RichText::new(&target.name).strong());
            ui.add(egui::Label::new(target.path.display().to_string()).wrap());
            ui.label(format!("Scanned: {} on disk · {} file size", self.size(target.allocated), self.size(target.logical)));
            ui.add_space(8.0);
            if permanent {
                if target.directory { ui.label("Deletes this folder and all its current contents, including files added after this scan."); }
                ui.label(RichText::new("This cannot be undone. You cannot restore it from the Recycle Bin.").color(Color32::from_rgb(239, 119, 105)));
            } else {
                if target.directory { ui.label("Moves this folder and all its current contents, including files added after this scan."); }
                ui.label("You can restore it from the Windows Recycle Bin. Recycled items still occupy disk space until the bin is emptied.");
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let cancel = ui.button("Cancel");
                if confirmation.focus_cancel { cancel.request_focus(); }
                let confirm = if permanent {
                    ui.button(RichText::new("Delete permanently").color(Color32::from_rgb(239, 119, 105))).clicked()
                } else {
                    ui.button(RichText::new("Move to Recycle Bin").color(AMBER)).clicked()
                };
                (cancel.clicked(), confirm)
            }).inner
        });
        if let Some(pending) = &mut self.delete_pending {
            pending.focus_cancel = false;
        }
        if modal.inner.0 || modal.should_close() {
            self.delete_pending = None;
        } else if modal.inner.1 {
            self.delete_pending = None;
            self.scan_error = None;
            self.scan_stale = true;
            let (sender, receiver) = mpsc::channel();
            self.deletion = Some(DeleteJob {
                generation: self.generation,
                target: target.clone(),
                method: confirmation.method,
                receiver,
            });
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = recycle::delete(&confirmation.target, confirmation.method);
                let _ = sender.send(result);
                ctx.request_repaint();
            });
        }
    }

    /// Builds the cached map: tile fills, reserved headers, folder outlines,
    /// then text labels. Returns the fill mesh and the label shapes.
    fn map_artwork(
        &self,
        ui: &egui::Ui,
        scan: &Scan,
        filtered: bool,
    ) -> (egui::Mesh, Vec<egui::Shape>) {
        let painter = ui.painter();
        let mut mesh = egui::Mesh::default();
        let mut labels = vec![];
        let pixels_per_point = ui.ctx().pixels_per_point();
        let expanded: HashSet<_> = self
            .map_cache
            .iter()
            .filter_map(|tile| {
                if tile.depth == 0 {
                    None
                } else if tile.kind == treemap::Kind::Aggregate {
                    Some(tile.id)
                } else {
                    scan.nodes[tile.id].parent
                }
            })
            .collect();
        let mut pattern_budget = 2048;
        for tile in &self.map_cache {
            let block = tile_rect(tile);
            if !block.is_positive() {
                continue;
            }
            let summarized = tile.kind == treemap::Kind::Aggregate
                || (tile.kind == treemap::Kind::Folder && !expanded.contains(&tile.id));
            let fill = match tile.kind {
                treemap::Kind::Folder if summarized => GROUPED,
                treemap::Kind::Folder => RAISED,
                treemap::Kind::Aggregate => GROUPED,
                treemap::Kind::File => {
                    muted_tile(file_color(&scan.nodes[tile.id].name), block.area())
                }
            };
            if tile.kind == treemap::Kind::File {
                add_file_surface(&mut mesh, block, fill, tile.id, pixels_per_point);
            } else {
                mesh.add_colored_rect(block, fill);
                if summarized {
                    add_group_pattern(&mut mesh, block, pixels_per_point, &mut pattern_budget);
                }
            }
        }
        // Headings occupy space reserved by the layout, never file content.
        for tile in self.map_cache.iter().filter(|tile| tile.header) {
            let block = tile_rect(tile);
            let heading =
                Rect::from_min_size(block.min, vec2(block.width(), treemap::HEADER_HEIGHT));
            mesh.add_colored_rect(heading, FOLDER_HEADER);
            mesh.add_colored_rect(
                Rect::from_min_size(
                    egui::pos2(block.left(), heading.bottom() - 1.0),
                    vec2(block.width(), 1.0),
                ),
                BORDER,
            );
        }
        // Paint ancestors last so their stronger perimeter survives where a
        // child shares an edge. Tiny folder outlines stay quiet.
        for tile in self.map_cache.iter().rev() {
            let block = tile_rect(tile);
            if tile.kind == treemap::Kind::Folder
                && tile.depth >= 1
                && block.width() >= 12.0
                && block.height() >= 12.0
                && block.area() >= 600.0
            {
                let width = if tile.depth <= 2 {
                    2.0
                } else if tile.header {
                    1.0
                } else {
                    0.5
                };
                add_outline(&mut mesh, block, FOLDER_EDGE, width);
            }
        }
        for tile in &self.map_cache {
            let block = tile_rect(tile);
            if !block.is_positive() {
                continue;
            }
            let node = &scan.nodes[tile.id];
            if tile.kind == treemap::Kind::Folder {
                if tile.header {
                    for (x, galley) in name_and_size(
                        painter,
                        &node.name,
                        &self.size(tile.size),
                        block.width() - 8.0,
                        FontId::proportional(12.0),
                        TEXT,
                    ) {
                        labels.push(egui::Shape::galley(
                            block.min + vec2(5.0 + x, 1.0),
                            galley,
                            TEXT,
                        ));
                    }
                }
                if !expanded.contains(&tile.id) {
                    let mut content = block;
                    if tile.header {
                        content.min.y += treemap::HEADER_HEIGHT;
                    }
                    add_group_badge(
                        painter,
                        &mut mesh,
                        &mut labels,
                        content,
                        tile.files,
                        filtered,
                    );
                }
                continue;
            }
            if tile.kind == treemap::Kind::Aggregate {
                add_group_badge(painter, &mut mesh, &mut labels, block, tile.files, filtered);
                continue;
            }
            // Full names identify files; repeated short prefixes do not.
            // Small tiles still expose their full details immediately on hover.
            if block.width() < 104.0 || block.height() < 30.0 {
                continue;
            }
            let fill = muted_tile(file_color(&node.name), block.area());
            let ink = tile_ink(fill);
            let title = node.name.clone();
            let galley = painter.layout_no_wrap(title, FontId::proportional(12.0), ink);
            if galley.size().x > block.width() - 10.0 {
                continue;
            }
            let at = block.min + vec2(5.0, 3.0);
            labels.push(egui::Shape::galley(at, galley, ink));
            if block.height() >= 48.0 {
                let galley =
                    painter.layout_no_wrap(self.size(tile.size), FontId::monospace(12.0), ink);
                let at = block.min + vec2(5.0, 19.0);
                if galley.size().x <= block.width() - 10.0 {
                    labels.push(egui::Shape::galley(at, galley, ink));
                }
            }
        }
        (mesh, labels)
    }

    fn breakdown(&mut self, ui: &mut egui::Ui, _scan: &Arc<Scan>) {
        if let Some(totals) = self.type_results(ui.ctx(), false) {
            self.types_cache = totals;
            self.types_key = Some((self.current, self.metric));
        } else {
            self.types_cache = vec![].into();
            self.types_key = None;
        }
        let types_loading = self.types_key.is_none();
        let totals = category_totals(&self.types_cache);
        let total = totals.iter().sum::<u64>();
        let (bar, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
        draw_breakdown(ui, bar, &totals, total);
        response.on_hover_text("Area follows the selected size metric. Colors identify file types. Click a type to isolate it; click it again or All types to clear.");
        let matching = self
            .weights
            .as_ref()
            .filter(|_| !self.query.is_empty() || self.category.is_some())
            .map(|weights| format!("{} matching", self.size(weights[self.current])));
        let show_matching = matching.is_some() && ui.available_width() >= 950.0;
        let reserved = if show_matching { 240.0 } else { 100.0 };
        let font = FontId::proportional(12.0);
        let full_width = Category::ALL
            .iter()
            .map(|category| {
                ui.painter()
                    .layout_no_wrap(category.label().into(), font.clone(), TEXT)
                    .size()
                    .x
                    + 26.0
            })
            .sum::<f32>();
        let compact = full_width + reserved > ui.available_width();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (i, category) in Category::ALL.iter().enumerate() {
                let title = if compact {
                    match category {
                        Category::Documents => "Docs",
                        Category::Applications => "Apps",
                        Category::Data => "Assets",
                        _ => category.label(),
                    }
                } else {
                    category.label()
                };
                let galley = ui
                    .painter()
                    .layout_no_wrap(title.into(), font.clone(), TEXT);
                let width = galley.size().x + 22.0;
                let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
                let active = self.category == Some(*category);
                let color = category_color(*category);
                if active || response.hovered() {
                    ui.painter().rect_filled(rect, 3, RAISED);
                }
                if active {
                    ui.painter().rect_stroke(
                        rect,
                        3,
                        Stroke::new(1.0_f32, color),
                        egui::StrokeKind::Inside,
                    );
                }
                ui.painter().rect_filled(
                    Rect::from_center_size(
                        egui::pos2(rect.left() + 8.0, rect.center().y),
                        vec2(10.0, 10.0),
                    ),
                    2,
                    color,
                );
                ui.painter().galley(
                    egui::pos2(rect.left() + 17.0, rect.center().y - galley.size().y * 0.5),
                    galley,
                    TEXT,
                );
                if response.clicked() {
                    self.category = if active { None } else { Some(*category) };
                    self.dirty = true;
                }
                response.on_hover_ui(|ui| {
                    let members = || {
                        self.types_cache.iter().filter(|(extension, _, _)| {
                            Category::from_extension(extension) == *category
                        })
                    };
                    ui.label(RichText::new(category.label()).strong());
                    ui.label(category.examples());
                    if types_loading {
                        ui.label("Preparing type totals…");
                        return;
                    }
                    ui.label(format!(
                        "{} · {:.1}% of this folder · {} files",
                        self.size(totals[i]),
                        share(totals[i], total),
                        count(members().map(|(_, _, files)| *files).sum())
                    ));
                    if *category == Category::Other {
                        let mut largest: Vec<_> = members().collect();
                        largest.sort_by_key(|(_, size, _)| std::cmp::Reverse(*size));
                        if !largest.is_empty() {
                            ui.separator();
                            ui.label("Largest unclassified types:");
                            for (extension, size, _) in largest.into_iter().take(6) {
                                ui.label(
                                    RichText::new(format!("{extension:<14}{}", self.size(*size)))
                                        .monospace()
                                        .size(12.0),
                                );
                            }
                        }
                    }
                    ui.label(
                        RichText::new("Click to isolate this type in every view.").color(MUTED),
                    );
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("All types")
                    .on_hover_text("Clear the type filter; keep the current search.")
                    .clicked()
                {
                    self.category = None;
                    self.dirty = true;
                }
                if self.filter_receiver.is_some() || types_loading {
                    ui.spinner();
                } else if show_matching && let Some(text) = &matching {
                    ui.label(RichText::new(text).size(12.0).color(MUTED));
                }
            });
        });
    }

    fn selection_strip(&mut self, ui: &mut egui::Ui, scan: &Arc<Scan>) {
        let id = self.selected.unwrap_or(self.current);
        let node = &scan.nodes[id];
        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let name_width = (ui.available_width() - 250.0).max(120.0);
                    ui.allocate_ui_with_layout(
                        vec2(name_width, 20.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                RichText::new(self.size(node.size(self.metric)))
                                    .monospace()
                                    .size(12.0),
                            );
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    entry_icon(
                                        ui,
                                        node.directory,
                                        if node.directory {
                                            FOLDER
                                        } else {
                                            file_color(&node.name)
                                        },
                                    );
                                    ui.add(
                                        egui::Label::new(RichText::new(&node.name).strong())
                                            .truncate(),
                                    )
                                    .on_hover_text(scan.path(id).display().to_string());
                                },
                            );
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Show in Explorer").clicked() {
                            reveal(&scan.path(id));
                        }
                        if ui.small_button("Copy path").clicked() {
                            ui.ctx().copy_text(scan.path(id).display().to_string());
                        }
                    });
                });
            });
    }

    /// Fraction of the NTFS index read so far, known only while that phase runs.
    fn scan_fraction(&self) -> Option<f32> {
        let total = self.progress.metadata_total.load(Ordering::Relaxed);
        (self.receiver.is_some() && self.progress.phase.load(Ordering::Relaxed) == 1 && total > 0)
            .then(|| {
                let done = self.progress.metadata_bytes.load(Ordering::Relaxed);
                (done as f32 / total as f32).clamp(0.0, 1.0)
            })
    }

    fn relaunch_as_admin(&mut self) {
        self.scan_error = None;
        self.message = match relaunch_admin() {
            Ok(()) => "Administrator window requested. Approve the Windows prompt to open it; no scan starts automatically.".into(),
            Err(error) => error,
        };
    }

    /// A banner for the last error or message. Buttons take their width first,
    /// so a long message wraps to two lines instead of pushing them off screen.
    fn notice(&mut self, ui: &mut egui::Ui, text: &str, offer_admin: bool, is_error: bool) {
        let accent = if is_error { AMBER } else { TEXT };
        egui::Frame::new()
            .fill(RAISED)
            .stroke(Stroke::new(1.0_f32, if is_error { AMBER } else { BORDER }))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // `with_layout` claims all remaining height unless it sits in a row.
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Dismiss").clicked() {
                            if is_error {
                                self.scan_error = None;
                            } else {
                                self.message.clear();
                            }
                        }
                        if offer_admin && ui.small_button("Relaunch as admin").clicked() {
                            self.relaunch_as_admin();
                        }
                        let mut job = egui::text::LayoutJob::simple(
                            text.to_owned(),
                            FontId::proportional(14.0),
                            accent,
                            ui.available_width(),
                        );
                        job.wrap.max_rows = 2;
                        // A horizontal row does not wrap text unless told to.
                        ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Wrap))
                            .on_hover_text(text);
                    });
                });
            });
        ui.add_space(4.0);
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K)) {
            self.search_focus = true;
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        let Some(scan) = self.scan.clone() else {
            return;
        };
        if ctx.input(|i| i.key_pressed(egui::Key::Backspace)) && self.current != 0 {
            self.navigate(scan.nodes[self.current].parent.unwrap_or(0));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(id) = self.selected
        {
            self.navigate(if scan.nodes[id].directory {
                id
            } else {
                scan.nodes[id].parent.unwrap_or(0)
            });
            if !scan.nodes[id].directory {
                self.select(id);
            }
        }
        if self.view != View::Folders {
            return;
        }
        for (key, direction) in [
            (egui::Key::ArrowLeft, vec2(-1.0, 0.0)),
            (egui::Key::ArrowRight, vec2(1.0, 0.0)),
            (egui::Key::ArrowUp, vec2(0.0, -1.0)),
            (egui::Key::ArrowDown, vec2(0.0, 1.0)),
        ] {
            if ctx.input(|i| i.key_pressed(key))
                && let Some(id) = neighbor(&self.map_cache, self.selected, direction)
            {
                self.select(id);
            }
        }
    }

    fn status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").exact_height(36.0).frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(12,5))).show(ctx,|ui| {ui.horizontal(|ui| {
            if let Some(job)=&self.io_job {
                ui.spinner(); ui.label(format!("{} · {} bytes",job.label,count(job.progress.metadata_bytes.load(Ordering::Relaxed))));
                if ui.add_enabled(!job.progress.cancelled(), egui::Button::new("Cancel job")).clicked() { job.progress.cancel.store(true,Ordering::Relaxed); }
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            if self.scan_stale {ui.label(RichText::new("Out of date · Rescan").color(AMBER));}
            if self.receiver.is_some() {
                ui.spinner();let phase=self.progress.phase.load(Ordering::Relaxed);
                ui.label(RichText::new(format!("{} · {:.1}s · {} files",match phase {1=>"Reading index",2=>"Building tree",3=>"Loading snapshot",_=>"Scanning"},self.started.elapsed().as_secs_f64(),count(self.progress.files.load(Ordering::Relaxed)))).size(12.0));
                if let Some(fraction)=self.scan_fraction() {ui.add(egui::ProgressBar::new(fraction).desired_width(140.0).desired_height(8.0).text(RichText::new(format!("{:.0}%",fraction*100.0)).size(10.0)));}
                ctx.request_repaint_after(Duration::from_millis(100));
            } else if let Some(scan)=self.scan.clone() {
                ui.label(RichText::new(result_state(&scan)).size(12.0).color(if scan.stats.cancelled {AMBER}else{MUTED}));
                ui.menu_button("Scan notes",|ui| {
                    ui.set_max_width(420.0);
                    ui.label(brand::RELEASE_LABEL);
                    ui.label(format!("{} · {:.2}s",result_state(&scan),scan.stats.elapsed_seconds));
                    if let Some(timing) = &scan.stats.mft_timings {
                        ui.label(format!("Captured scan timing · {} index attempt(s)", timing.index_attempts));
                        ui.label(format!("Index reads: {:.2}s · Parsing/validation: {:.2}s", timing.index_io_seconds, timing.parse_seconds));
                        ui.label(format!("External lists: {:.2}s · Extension merge: {:.2}s", timing.external_list_seconds, timing.extension_merge_seconds));
                        ui.label(format!("Tree: {:.2}s · Bootstrap: {:.2}s", timing.tree_seconds, timing.bootstrap_seconds));
                    }
                    if !is_saved_scan(&scan) && let Some(at)=self.loaded_at {
                        ui.label(if at.elapsed().as_secs()<60 {"Scanned just now".into()}else{format!("Scanned {} min ago",at.elapsed().as_secs()/60)});
                    }
                    ui.separator();
                    ui.label("Diskvale requires administrator access at launch. NTFS drives use the experimental MFT index reader; exFAT/FAT32 drives use a read-only folder scan.");
                    ui.label(format!("Scan engine: {}", scan.stats.engine));
                    ui.label("The index reader is experimental: matched whole-drive accuracy and speed validation remains pending.");
                    ui.separator();
                    ui.label(format!("{} read errors · {} shared links · {} unknown allocations",count(scan.stats.errors),count(scan.stats.shared_links),count(scan.stats.unknown_allocations)));
                    for notice in &scan.stats.notices {ui.separator();ui.label(notice);}
                    if let Some(space)=&scan.stats.volume_space {ui.separator();ui.label(format!("Free space changed by {} during the scan. Outside indexed files is a remainder, not a proven NTFS-metadata total.",self.size(space.free.abs_diff(space.free_at_start))));}
                    if let Some(delta)=self.delta(&scan,0) {ui.separator();ui.label(format!("Change vs saved scan: {}{}",if delta<0 {"−"}else{"+"},self.size(delta.unsigned_abs().min(u64::MAX as u128) as u64)));}
                });
            } else {ui.label(RichText::new(brand::READY_LABEL).size(12.0).color(MUTED));}
        });});
    }

    pub(super) fn render(&mut self, ctx: &egui::Context) {
        self.poll();
        if self.close_popups {
            egui::Popup::close_all(ctx);
            self.close_popups = false;
        }
        // Warm the first file list while the map is visible, using the same
        // keyed cache the Files view will read. This does not copy the scan.
        if self.prefetch_results && self.scan.is_some() {
            self.prefetch_results = false;
            self.rebuild_rows(ctx);
        }
        self.sync_selection();
        if self.delete_pending.is_none() && self.deletion.is_none() {
            self.keyboard(ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.receiver.is_some() {
            self.progress.cancel.store(true, Ordering::Relaxed);
        }
        self.header(ctx);
        self.status(ctx);
        self.sidebar(ctx);
        self.filters(ctx);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(10)).show(ctx,|ui| {
            if let Some(error)=self.scan_error.clone() {
                let (text,needs_admin)=friendly_error(&error,self.elevated);
                self.notice(ui,&text,needs_admin,true);
            } else if !self.message.is_empty() {
                let text=self.message.clone();
                self.notice(ui,&text,false,false);
            }
            if let Some(scan)=self.scan.clone() {
                self.actions(ui,&scan);
                if let Some(error)=Query::parse(&self.query).error {ui.label(RichText::new(error).color(AMBER));}
                let breakdown_height=36.0;
                let selection_height=34.0;
                let available=(ui.available_height()-breakdown_height-selection_height-12.0).max(60.0);
                if self.view==View::Folders {self.map(ui,&scan,available);}
                else {ui.allocate_ui_with_layout(vec2(ui.available_width(),available),egui::Layout::top_down(egui::Align::Min),|ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    self.table(ui,&scan,(available-28.0).max(40.0));
                });}
                ui.add_space(4.0);self.breakdown(ui,&scan);ui.add_space(4.0);self.selection_strip(ui,&scan);
            } else {
                ui.add_space((ui.available_height()*0.23).min(140.0));
                ui.vertical_centered(|ui| {
                    let (rect,_)=ui.allocate_exact_size(vec2(90.0,70.0),Sense::hover());
                    for (x,y,w,h,category) in [(0.,0.,52.,70.,Category::Media),(52.,0.,38.,32.,Category::Archives),(52.,32.,20.,38.,Category::Data),(72.,32.,18.,38.,Category::Applications)] {ui.painter().rect_filled(Rect::from_min_size(rect.min+vec2(x,y),vec2(w,h)),0,category_color(category));}
                    ui.add_space(18.0);ui.label(RichText::new(if self.receiver.is_some(){"Preparing your space map…"}else{"See your whole drive at a glance."}).size(26.0).strong());
                    ui.label(RichText::new("Every file. One map. A clear picture of your space.").size(15.0).color(MUTED));ui.add_space(18.0);
                    if self.receiver.is_some(){
                        match self.scan_fraction() {
                            Some(fraction)=>{ui.add(egui::ProgressBar::new(fraction).desired_width(320.0).text(format!("{:.0}%",fraction*100.0)));}
                            None=>{ui.spinner();}
                        }
                    }
                    else {
                        if ui.add(egui::Button::new(RichText::new(format!("Scan {}",self.path)).color(BG).strong()).fill(ACCENT).min_size(vec2(150.0,36.0))).clicked(){self.start_scan(ctx.clone());}
                        if ui.button("Open saved scan…").clicked() && let Some(path)=rfd::FileDialog::new().add_filter("Diskvale snapshot",&["json"]).pick_file(){self.load_scan(ctx.clone(),path);}
                    }
                    ui.add_space(10.0);ui.label(RichText::new("NTFS · exFAT · FAT32 · Internal or external drives").size(12.0).color(MUTED));
                    if !self.elevated && self.receiver.is_none() {
                        ui.add_space(6.0);
                        ui.label(RichText::new("Diskvale requires administrator access to open.").size(12.0).color(MUTED));
                        if ui.button("Relaunch as admin").on_hover_text("Opens a new Diskvale window after the Windows prompt. No scan starts automatically.").clicked() {self.relaunch_as_admin();}
                    }
                });
            }
        });
        self.delete_dialog(ctx);
    }
}

/// Visible provenance comes from runtime state, never from snapshot notices.
fn result_state(scan: &Scan) -> &'static str {
    use canopy::model::Provenance;
    if scan.stats.cancelled {
        "Partial scan"
    } else {
        match scan.stats.runtime.provenance() {
            Provenance::Live => "Live scan",
            Provenance::Directory => "Live folder scan · Read only",
            Provenance::Snapshot => "Saved result · Read only",
            Provenance::Replay => "Saved index · Read only",
            Provenance::Incomplete => "Partial scan",
        }
    }
}

/// Folder outlines in the map: dark enough to read against every category colour.
const FOLDER_EDGE: Color32 = Color32::from_rgb(12, 16, 20);
/// Neutral header bands separate folder names from colored file content.
const FOLDER_HEADER: Color32 = Color32::from_rgb(30, 40, 48);
/// Largest number of children the folder tree lists per directory.
const TREE_FANOUT: usize = 256;

/// Indent grows 12 px per level, then tightens so deep paths stay on screen.
fn tree_indent(depth: usize) -> f32 {
    depth.min(6) as f32 * 12.0 + depth.saturating_sub(6).min(8) as f32 * 4.0
}

/// Scan errors keep their technical text; the common permission failure becomes
/// plain advice, and says whether a relaunch could fix it.
fn friendly_error(error: &str, elevated: bool) -> (String, bool) {
    if !elevated && error.contains("denied access to the NTFS index") {
        (
            "Windows blocked access to this drive's NTFS index. Diskvale requires administrator access; start it with Run as administrator.".into(),
            true,
        )
    } else {
        (error.to_owned(), false)
    }
}

/// `YYYY-MM-DD` in UTC, or a dash when the scan did not record a date.
pub(super) fn modified_day(unix: u64) -> String {
    if unix == 0 {
        "—".into()
    } else {
        modified(unix).trim_end_matches(" UTC").to_owned()
    }
}

/// Large files get a cushion; smaller files still keep their real boundaries.
const FILE_EDGE_MIN: f32 = 24.0;

/// The fill darkened just enough to read as an edge against it.
fn tile_edge(fill: Color32) -> Color32 {
    let darker = |value: u8| (value as f32 * 0.78) as u8;
    Color32::from_rgb(darker(fill.r()), darker(fill.g()), darker(fill.b()))
}

/// An opaque, nine-vertex cushion stays in the cached fill mesh. The center is
/// the category color; corners darken by 14%, with a gentler edge midpoint.
fn add_cushion(mesh: &mut egui::Mesh, block: Rect, fill: Color32) {
    let tone = |factor: f32| {
        Color32::from_rgb(
            (fill.r() as f32 * factor) as u8,
            (fill.g() as f32 * factor) as u8,
            (fill.b() as f32 * factor) as u8,
        )
    };
    let start = mesh.vertices.len() as u32;
    let xs = [block.left(), block.center().x, block.right()];
    let ys = [block.top(), block.center().y, block.bottom()];
    for (row, y) in ys.into_iter().enumerate() {
        for (column, x) in xs.into_iter().enumerate() {
            let factor = match (row == 1, column == 1) {
                (true, true) => 1.0,
                (true, false) | (false, true) => 0.94,
                (false, false) => 0.86,
            };
            mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::pos2(x, y),
                uv: egui::epaint::WHITE_UV,
                color: tone(factor),
            });
        }
    }
    for row in 0..2 {
        for column in 0..2 {
            let top = start + row * 3 + column;
            mesh.indices
                .extend_from_slice(&[top, top + 3, top + 1, top + 1, top + 3, top + 4]);
        }
    }
}

/// Each edge belongs to an actual file. Subpixel files get a slight stable tone
/// variation rather than invented divisions or an expensive per-file widget.
fn add_file_surface(
    mesh: &mut egui::Mesh,
    block: Rect,
    fill: Color32,
    id: usize,
    pixels_per_point: f32,
) {
    let pixel = 1.0 / pixels_per_point;
    let small = block.width().min(block.height()) * pixels_per_point;
    let cushioned = block.width() >= FILE_EDGE_MIN && block.height() >= FILE_EDGE_MIN;
    if cushioned {
        add_cushion(mesh, block, fill);
    } else {
        let factor = if small < 6.0 {
            0.94 + ((id.wrapping_mul(37) % 5) as f32 * 0.03)
        } else {
            1.0
        };
        mesh.add_colored_rect(block, tone(fill, factor));
    }
    if small >= 3.0 {
        let edge = tile_edge(fill);
        // Snap inside the original rectangle; never change its layout or area.
        let bottom = ((block.bottom() * pixels_per_point).floor() * pixel)
            .max(block.top() + pixel)
            .min(block.bottom());
        let right = ((block.right() * pixels_per_point).floor() * pixel)
            .max(block.left() + pixel)
            .min(block.right());
        mesh.add_colored_rect(
            Rect::from_min_max(
                egui::pos2(block.left(), bottom - pixel),
                egui::pos2(block.right(), bottom),
            ),
            edge,
        );
        mesh.add_colored_rect(
            Rect::from_min_max(
                egui::pos2(right - pixel, block.top()),
                egui::pos2(right, bottom - pixel),
            ),
            edge,
        );
    }
    if cushioned {
        let highlight = Color32::from_rgb(
            fill.r().saturating_add(12),
            fill.g().saturating_add(12),
            fill.b().saturating_add(12),
        );
        mesh.add_colored_rect(
            Rect::from_min_size(block.min, vec2(block.width(), pixel * 0.65)),
            highlight,
        );
    }
}

fn tone(fill: Color32, factor: f32) -> Color32 {
    Color32::from_rgb(
        (fill.r() as f32 * factor).min(255.0) as u8,
        (fill.g() as f32 * factor).min(255.0) as u8,
        (fill.b() as f32 * factor).min(255.0) as u8,
    )
}

/// Clipped diagonal bands distinguish summaries without suggesting fake files.
/// A global band budget bounds the extra cached geometry on a full-drive map.
fn add_group_pattern(
    mesh: &mut egui::Mesh,
    block: Rect,
    pixels_per_point: f32,
    budget: &mut usize,
) {
    if block.width().min(block.height()) * pixels_per_point < 4.0 || *budget == 0 {
        return;
    }
    let span = block.width() + block.height();
    let spacing = (12.0 / pixels_per_point).max(span / 96.0);
    let thickness = 1.0 / pixels_per_point;
    let mut offset = spacing * 0.5;
    while offset < span && *budget > 0 {
        let mut polygon = vec![
            egui::Pos2::ZERO,
            egui::pos2(block.width(), 0.0),
            egui::pos2(block.width(), block.height()),
            egui::pos2(0.0, block.height()),
        ];
        for (limit, above) in [(offset, true), (offset + thickness, false)] {
            let mut clipped = Vec::with_capacity(6);
            let mut previous = *polygon.last().unwrap();
            for point in polygon {
                let a = previous.x + previous.y - limit;
                let b = point.x + point.y - limit;
                let a_inside = if above { a >= 0.0 } else { a <= 0.0 };
                let b_inside = if above { b >= 0.0 } else { b <= 0.0 };
                if a_inside != b_inside {
                    let t = a / (a - b);
                    clipped.push(previous + (point - previous) * t);
                }
                if b_inside {
                    clipped.push(point);
                }
                previous = point;
            }
            polygon = clipped;
            if polygon.is_empty() {
                break;
            }
        }
        let start = mesh.vertices.len() as u32;
        for point in &polygon {
            let position = block.min + point.to_vec2();
            mesh.colored_vertex(block.clamp(position), Color32::from_rgb(63, 78, 92));
        }
        for index in 1..polygon.len().saturating_sub(1) {
            mesh.add_triangle(start, start + index as u32, start + index as u32 + 1);
        }
        *budget -= 1;
        offset += spacing;
    }
}

fn compact_files(files: u64) -> String {
    if files < 1000 {
        files.to_string()
    } else if files < 1_000_000 {
        format!("{:.1}k", files as f64 / 1000.0)
    } else {
        format!("{:.1}m", files as f64 / 1_000_000.0)
    }
}

/// A readable count takes priority over the stack icon. Only tiny regions or
/// counts that cannot fit use the icon alone; generous regions use larger type.
fn add_group_badge(
    painter: &egui::Painter,
    mesh: &mut egui::Mesh,
    labels: &mut Vec<egui::Shape>,
    block: Rect,
    files: u64,
    filtered: bool,
) {
    if block.width() < 20.0 || block.height() < 23.0 {
        return;
    }
    let number = compact_files(files);
    let short_number = if files >= 1_000_000 {
        format!("{:.0}m", files as f64 / 1_000_000.0)
    } else if files >= 1000 {
        format!("{:.0}k", files as f64 / 1000.0)
    } else {
        files.to_string()
    };
    let candidates = [
        format!(
            "{} {}file{}",
            count(files),
            if filtered { "matching " } else { "" },
            if files == 1 { "" } else { "s" }
        ),
        format!("{number} file{}", if files == 1 { "" } else { "s" }),
        number,
        short_number,
    ];
    let font_size = if block.width() >= 120.0 && block.height() >= 64.0 {
        16.0
    } else if block.width() >= 72.0 && block.height() >= 40.0 {
        14.0
    } else {
        12.0
    };
    let padding = 3.0;
    let available = block.width() - 6.0 - padding * 2.0;
    let galley = if block.area() >= 800.0 && block.width() >= 28.0 && block.height() >= 26.0 {
        candidates
            .into_iter()
            .map(|text| painter.layout_no_wrap(text, FontId::proportional(font_size), TEXT))
            .find(|galley| {
                galley.size().x <= available
                    && galley.size().y + padding * 2.0 <= block.height() - 6.0
            })
    } else {
        None
    };
    let with_icon = galley
        .as_ref()
        .is_none_or(|text| text.size().x + 14.0 <= available);
    let width = galley.as_ref().map_or(14.0, |text| {
        text.size().x + padding * 2.0 + if with_icon { 14.0 } else { 0.0 }
    });
    let height = galley
        .as_ref()
        .map_or(17.0, |text| (text.size().y + padding * 2.0).ceil());
    let badge = Rect::from_min_size(block.min + vec2(3.0, 3.0), vec2(width, height));
    mesh.add_colored_rect(badge, Color32::from_rgb(27, 36, 45));
    if with_icon {
        let back = Rect::from_min_size(badge.min + vec2(3.0, (height - 9.0) * 0.5), vec2(7.0, 6.0));
        let front = back.translate(vec2(2.0, 3.0));
        add_outline(mesh, back, MUTED, 0.75);
        mesh.add_colored_rect(front, Color32::from_rgb(27, 36, 45));
        add_outline(mesh, front, TEXT, 0.75);
    }
    if let Some(galley) = galley {
        labels.push(egui::Shape::galley(
            badge.min + vec2(padding + if with_icon { 14.0 } else { 0.0 }, padding),
            galley,
            TEXT,
        ));
    }
}

/// Folder frames stay inside their byte-proportional regions.
fn add_outline(mesh: &mut egui::Mesh, block: Rect, color: Color32, width: f32) {
    let (w, h) = (block.width(), block.height());
    for edge in [
        Rect::from_min_size(block.min, vec2(w, width)),
        Rect::from_min_size(egui::pos2(block.min.x, block.max.y - width), vec2(w, width)),
        Rect::from_min_size(block.min, vec2(width, h)),
        Rect::from_min_size(egui::pos2(block.max.x - width, block.min.y), vec2(width, h)),
    ] {
        mesh.add_colored_rect(edge, color);
    }
}

/// Lays out "name · size" as pieces with their x offsets. The name shortens
/// first, so a narrow header still reports its weight. When even a short name
/// will not leave room for the size, the name stands alone: a bare number says
/// nothing, and the tooltip carries the size.
fn name_and_size(
    painter: &egui::Painter,
    name: &str,
    size: &str,
    width: f32,
    font: FontId,
    ink: Color32,
) -> Vec<(f32, Arc<egui::Galley>)> {
    let suffix = painter.layout_no_wrap(format!(" · {size}"), font.clone(), ink);
    let full = painter.layout_no_wrap(name.to_owned(), font.clone(), ink);
    let one_row = |text: String, width: f32| {
        let mut job = egui::text::LayoutJob::simple(text, font.clone(), ink, width.max(1.0));
        job.wrap.max_rows = 1;
        painter.layout_job(job)
    };
    if full.size().x + suffix.size().x <= width {
        let x = full.size().x;
        return vec![(0.0, full), (x, suffix)];
    }
    let room = width - suffix.size().x;
    if room >= 28.0 {
        let name = one_row(name.to_owned(), room);
        let x = name.size().x;
        return vec![(0.0, name), (x, suffix)];
    }
    vec![(0.0, one_row(name.to_owned(), width))]
}

fn share(value: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        value as f64 / total as f64 * 100.0
    }
}
fn tile_rect(tile: &treemap::Tile) -> Rect {
    Rect::from_min_size(
        egui::pos2(tile.rect.x, tile.rect.y),
        vec2(tile.rect.w, tile.rect.h),
    )
}
fn ancestor_set(scan: &Scan, id: usize) -> HashSet<usize> {
    let mut set = HashSet::new();
    let mut id = Some(id);
    while let Some(i) = id {
        set.insert(i);
        id = scan.nodes[i].parent;
    }
    set
}
fn category_totals(extensions: &[(String, u64, u64)]) -> [u64; 8] {
    let mut result = [0u64; 8];
    for (ext, size, _) in extensions {
        let category = Category::from_extension(ext);
        let i = Category::ALL.iter().position(|c| *c == category).unwrap();
        result[i] = result[i].saturating_add(*size);
    }
    result
}
fn map_pointer(response: &egui::Response) -> Option<egui::Pos2> {
    // Noninteractive tooltip widgets can cover last frame's hit-test target.
    // Check the map's layer instead, while still respecting menus and dialogs.
    response
        .ctx
        .rect_contains_pointer(response.layer_id, response.interact_rect)
        .then(|| response.ctx.pointer_hover_pos())
        .flatten()
}
fn instant_map_tooltip(
    response: &egui::Response,
    key: impl std::hash::Hash,
    content: impl FnOnce(&mut egui::Ui),
) {
    let Some(pointer) = map_pointer(response) else {
        return;
    };
    let bounds = response.ctx.content_rect();
    // Open toward the room there is: flip left or up near the right and bottom
    // edges, so the tooltip never covers the tile being inspected.
    let flip_x = pointer.x + 14.0 + 360.0 > bounds.right();
    let flip_y = pointer.y + 18.0 + 150.0 > bounds.bottom();
    let pivot = match (flip_x, flip_y) {
        (false, false) => Align2::LEFT_TOP,
        (true, false) => Align2::RIGHT_TOP,
        (false, true) => Align2::LEFT_BOTTOM,
        (true, true) => Align2::RIGHT_BOTTOM,
    };
    let offset = vec2(
        if flip_x { -14.0 } else { 14.0 },
        if flip_y { -14.0 } else { 18.0 },
    );
    // An Area remembers its measured size and caps the next frame's width at it,
    // so one shared id lets a narrow tooltip squeeze every later one into
    // mid-word wraps. An id per hovered entry measures each tooltip afresh.
    egui::Area::new(response.id.with(("instant-map-tooltip", key)))
        .order(egui::Order::Tooltip)
        .movable(false)
        .interactable(false)
        .fade_in(false)
        .pivot(pivot)
        .fixed_pos(pointer + offset)
        .default_width(480.0)
        .constrain_to(bounds)
        .show(&response.ctx, |ui| {
            // Measure and paint a newly opened tooltip in the same frame.
            if ui.is_sizing_pass() {
                ui.ctx().request_discard("Measure instant map tooltip");
            }
            ui.style_mut().interaction.selectable_labels = false;
            egui::Frame::popup(ui.style()).show(ui, content);
        });
}
fn draw_breakdown(ui: &egui::Ui, bar: Rect, totals: &[u64; 8], total: u64) {
    ui.painter().rect_filled(bar, 0, RAISED);
    let mut x = bar.left();
    for (i, size) in totals.iter().enumerate() {
        let width = bar.width() * (*size as f32 / total.max(1) as f32);
        ui.painter().rect_filled(
            Rect::from_min_size(egui::pos2(x, bar.top()), vec2(width, bar.height())),
            0,
            category_color(Category::ALL[i]),
        );
        x += width;
    }
}
fn muted_tile(base: Color32, area: f32) -> Color32 {
    let amount = if area < 25.0 {
        0.45
    } else if area < 150.0 {
        0.25
    } else {
        0.0
    };
    let gray =
        (base.r() as f32 * 0.2126 + base.g() as f32 * 0.7152 + base.b() as f32 * 0.0722) as u8;
    let mix = |v: u8| ((v as f32 * (1.0 - amount)) + (gray as f32 * amount)) as u8;
    Color32::from_rgb(mix(base.r()), mix(base.g()), mix(base.b()))
}
fn neighbor(
    tiles: &[treemap::Tile],
    selected: Option<usize>,
    direction: egui::Vec2,
) -> Option<usize> {
    let source = tiles
        .iter()
        .rev()
        .find(|t| Some(t.id) == selected && t.kind != treemap::Kind::Aggregate);
    let Some(source) = source else {
        return tiles
            .iter()
            .find(|t| t.kind == treemap::Kind::File)
            .map(|t| t.id);
    };
    let center = tile_rect(source).center();
    tiles
        .iter()
        .filter(|t| {
            t.id != source.id
                && t.kind != treemap::Kind::Aggregate
                && t.depth >= source.depth
                && t.rect.w >= 2.0
                && t.rect.h >= 2.0
        })
        .filter_map(|t| {
            let delta = tile_rect(t).center() - center;
            let forward = delta.dot(direction);
            (forward > 0.0).then_some((
                t.id,
                forward + delta.dot(vec2(-direction.y, direction.x)).abs() * 3.0,
            ))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}
fn modified(unix: u64) -> String {
    if unix == 0 {
        return "Unavailable".into();
    }
    let days = (unix / 86400) as i64 + 719468;
    let era = days / 146097;
    let day = days - era * 146097;
    let year = (day - day / 1460 + day / 36524 - day / 146096) / 365;
    let doy = day - (365 * year + year / 4 - year / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = year + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} UTC")
}

pub(super) fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}
fn relaunch_admin() -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let file = scanner::windows::wide(&exe);
        let verb: Vec<u16> = "runas\0".encode_utf16().collect();
        // Explicit user action only. No scan arguments, token changes or auto-elevation.
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        } as isize;
        if result > 32 {
            Ok(())
        } else {
            Err("Administrator launch was cancelled or unavailable. You can continue viewing saved results.".into())
        }
    }
    #[cfg(not(windows))]
    {
        Err("Administrator launch requires Windows.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canopy::model::{Node, ScanStats, VolumeSpace};
    #[test]
    fn accepting_replacement_scan_before_arrow_input_clears_all_old_actions() {
        for same_length in [false, true] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let size = vec2(1050.0, 700.0);
            let mut app = app(Some(fixture()));
            settle(&ctx, &mut app, size);
            assert!(app.map_cache.iter().any(|t| t.id == 7));
            app.map_menu = Some(7);
            egui::Popup::open_id(&ctx, egui::Id::new("old-map-popup"));
            assert!(egui::Popup::is_any_open(&ctx));
            app.selected = None;
            let mut replacement = if same_length {
                fixture()
            } else {
                Scan {
                    root: "C:\\".into(),
                    nodes: vec![Node::new("root".into(), None, true)],
                    stats: Default::default(),
                }
            };
            if same_length {
                replacement.nodes[7].name = "different-file.bin".into();
            }
            let (sender, receiver) = mpsc::channel();
            app.receiver = Some(receiver);
            sender.send((app.scan_request, Ok(replacement))).unwrap();
            let _ = ctx.run(
                frame_input(
                    size,
                    vec![egui::Event::Key {
                        key: egui::Key::ArrowRight,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                ),
                |ctx| app.render(ctx),
            );
            assert!(
                app.selected.is_none(),
                "Old map selected an unrelated replacement node"
            );
            assert!(app.map_menu.is_none());
            assert!(!egui::Popup::is_any_open(&ctx));
            assert!(app.delete_pending.is_none());
            assert_eq!(app.generation, 1);
        }
    }
    #[test]
    fn late_old_scan_request_and_retained_confirmation_are_harmless() {
        let mut app = app(Some(fixture()));
        let target = recycle::Target::from_scan(app.scan.as_ref().unwrap(), 5).unwrap();
        app.delete_pending = Some(DeleteConfirmation::new(target, recycle::Method::RecycleBin));
        app.install_scan(fixture());
        let installed = app.scan.clone().unwrap();
        let (sender, receiver) = mpsc::channel();
        app.receiver = Some(receiver);
        app.scan_request = 2;
        sender.send((1, Ok(fixture()))).unwrap();
        app.poll();
        assert!(Arc::ptr_eq(app.scan.as_ref().unwrap(), &installed));
        assert!(app.delete_pending.is_none());
        assert_eq!(app.generation, 1);
    }
    #[test]
    fn blocked_io_worker_leaves_frames_and_navigation_responsive_and_discards_old_comparison() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        let size = vec2(1050.0, 700.0);
        let (started, startup) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        app.start_io(&ctx, "Blocked comparison", move |progress| {
            started.send(()).unwrap();
            blocked.recv().unwrap();
            assert!(progress.cancelled());
            Ok(IoResult::Baseline(Arc::new(fixture()), HashMap::new()))
        });
        startup.recv_timeout(Duration::from_secs(2)).unwrap();
        let start = Instant::now();
        for _ in 0..20 {
            let _ = ctx.run(frame_input(size, vec![]), |ctx| app.render(ctx));
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "Blocked writer blocked UI frames: {elapsed:?}"
        );
        click_label(&ctx, &mut app, size, "Files");
        assert!(app.view == View::Largest);
        app.start_io(&ctx, "Unbounded second job", |_| {
            panic!("Second job started")
        });
        app.install_scan(fixture());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.io_job.is_some() {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(app.baseline.is_none());
        assert!(app.message.contains("discarded"));
        eprintln!("Responsiveness: 20 egui frames with blocked IO writer: {elapsed:?}");
    }
    #[test]
    fn io_cancel_error_disconnect_and_old_export_are_reported() {
        let ctx = egui::Context::default();
        let mut app = app(Some(fixture()));
        for cancelled in [false, true] {
            let (sender, receiver) = mpsc::channel();
            let progress = Arc::new(Progress::default());
            progress.cancel.store(cancelled, Ordering::Relaxed);
            app.io_job = Some(IoJob {
                generation: 0,
                label: "fixture",
                progress,
                receiver,
            });
            if cancelled {
                sender.send(Err("cancelled fixture".into())).unwrap();
            } else {
                drop(sender);
            }
            app.poll();
            assert!(app.io_job.is_none());
            assert!(app.message.contains(if cancelled {
                "cancelled"
            } else {
                "disconnected"
            }));
        }
        let (sender, receiver) = mpsc::channel();
        app.io_job = Some(IoJob {
            generation: 0,
            label: "fixture",
            progress: Arc::default(),
            receiver,
        });
        app.install_scan(fixture());
        sender
            .send(Ok(IoResult::Exported("fixture.csv".into())))
            .unwrap();
        app.poll();
        assert!(app.message.contains("previous scan"));
        app.start_io(&ctx, "Injected error", |_| Err("fixture failure".into()));
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.io_job.is_some() {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(app.message.contains("fixture failure"));
    }
    #[test]
    fn large_generated_snapshot_job_shares_scan_and_validates_off_thread() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("large.json");
        let mut scan = Scan {
            root: "C:\\".into(),
            stats: Default::default(),
            nodes: vec![Node::new("root".into(), None, true)],
        };
        for i in 0..20_000 {
            let mut node = Node::new(format!("generated-{i:05}.dat"), Some(0), false);
            node.logical = 100;
            node.allocated = 4096;
            scan.nodes.push(node);
            scan.nodes[0].children.push(i + 1);
        }
        scan.finish();
        let mut app = app(Some(scan));
        let shared = app.scan.clone().unwrap();
        let ctx = egui::Context::default();
        let expected = shared.clone();
        let output = path.clone();
        app.start_io(&ctx, "Large snapshot", move |progress| {
            assert!(Arc::ptr_eq(&shared, &expected));
            canopy::output::snapshot(&shared, &output, &progress)?;
            let loaded = read_snapshot(&output, progress)?;
            assert_eq!(loaded.nodes.len(), 20_001);
            assert_eq!(loaded.stats.schema_version, 2);
            assert!(!loaded.deletion_eligible());
            Ok(IoResult::Saved(output))
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.io_job.is_some() {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(app.message.contains("saved"), "{}", app.message);
        assert!(path.metadata().unwrap().len() > 1_000_000);
    }
    #[test]
    fn every_empty_overflow_directory_is_reachable_in_bounded_pages() {
        for count in [300usize, 620] {
            let mut scan = Scan {
                root: "C:\\".into(),
                stats: Default::default(),
                nodes: vec![Node::new("root".into(), None, true)],
            };
            for i in 0..count {
                let id = scan.nodes.len();
                scan.nodes
                    .push(Node::new(format!("folder-{i:03}"), Some(0), true));
                scan.nodes[0].children.push(id);
            }
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let size = vec2(1050.0, 700.0);
            let mut app = app(Some(scan));
            app.selected = None;
            app.show_zero_entries = true;
            let mut seen = HashSet::new();
            for _ in 0..count.div_ceil(TREE_FANOUT) {
                app.tree_dirty = true;
                settle(&ctx, &mut app, size);
                let ids: Vec<_> = app
                    .tree_rows
                    .iter()
                    .filter_map(|r| {
                        if let TreeRow::Entry { id, .. } = r {
                            Some(*id)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert!(ids.len() <= TREE_FANOUT);
                let next_page = app
                    .tree_rows
                    .iter()
                    .find_map(|row| match row {
                        TreeRow::More { next_page, .. } => Some(*next_page),
                        _ => None,
                    })
                    .unwrap();
                for id in ids {
                    seen.insert(id);
                    app.navigate(id);
                    assert_eq!(app.current, id);
                    app.navigate(0);
                }
                // Exercise the page value supplied by the real More action.
                app.tree_pages.insert(0, next_page);
            }
            assert_eq!(app.tree_pages[&0], 0);
            assert_eq!(seen.len(), count);
            app.select(count);
            app.tree_pages.insert(0, 0);
            app.tree_dirty = true;
            settle(&ctx, &mut app, size);
            assert!(
                app.tree_rows
                    .iter()
                    .any(|r| matches!(r, TreeRow::Entry { id, .. } if *id==count))
            );
            assert!(app.tree_rows.len() <= TREE_FANOUT + 2);
        }
    }
    fn fixture() -> Scan {
        let mut scan = Scan {
            root: "C:\\".into(),
            stats: ScanStats {
                files: 152_420,
                folders: 580_500,
                elapsed_seconds: 6.13,
                engine: "Experimental NTFS MFT".into(),
                volume_space: Some(VolumeSpace {
                    capacity: 100 << 30,
                    free: 20 << 30,
                    free_at_start: 21 << 30,
                    captured_unix: 1_790_812_800,
                }),
                ..Default::default()
            },
            nodes: vec![
                Node::new("C:\\".into(), None, true),
                Node::new("Users".into(), Some(0), true),
                Node::new("Guest".into(), Some(1), true),
                Node::new("Desktop".into(), Some(2), true),
                Node::new("Projects".into(), Some(3), true),
                Node::new("capture.mp4".into(), Some(4), false),
                Node::new("capture-archive.zip".into(), Some(4), false),
                Node::new("pagefile.sys".into(), Some(0), false),
            ],
        };
        scan.nodes[0].children = vec![1, 7];
        for i in 1..4 {
            scan.nodes[i].children = vec![i + 1];
        }
        scan.nodes[4].children = vec![5, 6];
        for (id, size) in [(5, 40u64), (6, 25), (7, 10)] {
            scan.nodes[id].allocated = size << 30;
            scan.nodes[id].logical = (size + 1) << 30;
        }
        scan.finish();
        scan.stats.runtime = crate::model::RuntimeState::new(crate::model::Provenance::Live);
        scan.stats.directory_streams_captured = true;
        scan.stats.volume_serial = Some(123);
        for (i, node) in scan.nodes.iter_mut().enumerate() {
            node.reference = Some((1 << 48) | i as u64);
        }
        scan
    }
    fn app(scan: Option<Scan>) -> Canopy {
        Canopy {
            io_job: None,
            generation: 0,
            scan_request: 0,
            receiver_request: 0,
            close_popups: false,
            tree_pages: HashMap::new(),
            path: "C:\\".into(),
            scan: scan.map(Arc::new),
            current: 0,
            selected: Some(4),
            metric: Metric::Allocated,
            view: View::Folders,
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
            mft_file: None,
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
            loaded_at: Some(Instant::now()),
            elevated: false,
            initial_focus: None,
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
        }
    }
    #[test]
    fn preview_version_and_result_provenance_are_visible_without_starting_a_scan() {
        use crate::model::{Provenance, RuntimeState};
        for size in [vec2(1050.0, 700.0), vec2(1400.0, 900.0)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let input = || egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            let mut idle = app(None);
            let output = ctx.run(input(), |ctx| idle.render(ctx));
            assert!(
                texts(&output)
                    .iter()
                    .any(|(text, _, _)| text == brand::READY_LABEL)
            );
            assert!(idle.scan.is_none() && idle.receiver.is_none());
            for (provenance, cancelled, label) in [
                (Provenance::Live, false, "Live scan"),
                (Provenance::Directory, false, "Live folder scan · Read only"),
                (Provenance::Snapshot, false, "Saved result · Read only"),
                (Provenance::Replay, false, "Saved index · Read only"),
                (Provenance::Incomplete, true, "Partial scan"),
            ] {
                let mut scan = fixture();
                scan.stats.runtime = RuntimeState::new(provenance);
                scan.stats.cancelled = cancelled;
                assert_eq!(
                    is_saved_scan(&scan),
                    matches!(provenance, Provenance::Snapshot | Provenance::Replay)
                );
                if provenance != Provenance::Live {
                    assert!(!scan.deletion_eligible());
                }
                let mut result = app(Some(scan));
                let output = ctx.run(input(), |ctx| result.render(ctx));
                assert!(
                    texts(&output)
                        .iter()
                        .any(|(text, rect, _)| text == label && rect.right() <= size.x)
                );
                if provenance == Provenance::Live {
                    result.scan_stale = true;
                    let output = ctx.run(input(), |ctx| result.render(ctx));
                    assert!(
                        texts(&output)
                            .iter()
                            .any(|(text, _, _)| text == "Out of date · Rescan")
                    );
                }
            }
        }
    }

    #[test]
    fn views_fit_both_sizes_and_map_selection_expands_the_matching_tree_path() {
        for size in [vec2(1050.0, 700.0), vec2(1400.0, 900.0)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut app = app(Some(fixture()));
            app.scan_error = Some("Scan failed because this deliberately long message must wrap inside its banner beside Dismiss without widening the map or hiding its controls. ".repeat(4));
            let input = || egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            for view in [View::Folders, View::Largest, View::Types] {
                app.view = view;
                app.dirty = true;
                for _ in 0..2 {
                    let _ = ctx.run(input(), |ctx| app.render(ctx));
                }
                let output = ctx.run(input(), |ctx| app.render(ctx));
                for label in ["Recycle Bin", "All types", "Show in Explorer", "Copy path"] {
                    let bounds = output
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Text(t) if t.galley.job.text == label => {
                                Some(t.visual_bounding_rect())
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| {
                            panic!(
                                "Control {label} was not drawn at {size:?} (view index {})",
                                view as usize
                            )
                        });
                    assert!(
                        bounds.right() < size.x && bounds.bottom() < size.y,
                        "Control {label} clipped at {size:?}: {bounds:?}"
                    );
                }
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        let visible = text.visual_bounding_rect().intersect(shape.clip_rect);
                        assert!(
                            visible.right() <= size.x + 1.0 && visible.bottom() <= size.y + 1.0,
                            "Overflow {:?} at {size:?}: {visible:?}",
                            text.galley.job.text
                        );
                    }
                }
                assert!(output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="Show in Explorer" && t.visual_bounding_rect().bottom()<size.y-36.0)),"Missing visible details actions at {size:?}");
                assert!(output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="Show in Explorer" && t.visual_bounding_rect().bottom()>size.y-155.0)),"Short tables moved details out of the bottom bar at {size:?}");
            }
            assert!((0..=3).all(|id| app.expanded.contains(&id)));
            assert!(
                app.tree_rows
                    .iter()
                    .any(|row| matches!(row, TreeRow::Entry { id: 4, .. }))
            );
            app.view = View::Folders;
            let _ = ctx.run(input(), |ctx| app.render(ctx));
            let tile = *app.map_cache.iter().find(|t| t.id == 5).unwrap();
            let pos = tile_rect(&tile).center();
            ctx.style_mut(|style| style.interaction.tooltip_delay = 10.0);
            // The first pointer frame must expose useful file information even
            // with a long normal tooltip delay; hovering must not select it.
            for (id, size) in [(5, "44.02 GB"), (6, "27.92 GB")] {
                let point = tile_rect(app.map_cache.iter().find(|t| t.id == id).unwrap()).center();
                let output = ctx.run(
                    egui::RawInput {
                        events: vec![egui::Event::PointerMoved(point)],
                        ..input()
                    },
                    |ctx| app.render(ctx),
                );
                for text in [
                    "Location: C:\\Users\\Guest\\Desktop\\Projects".to_owned(),
                    format!("File size: {size}"),
                ] {
                    assert!(
                        output.shapes.iter().any(
                            |s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text==text)
                        ),
                        "Missing first-frame tooltip {text} at {size:?}"
                    );
                }
                assert_eq!(app.selected, Some(4));
            }
            let output = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::PointerMoved(egui::pos2(100.0, 60.0))],
                    ..input()
                },
                |ctx| app.render(ctx),
            );
            assert!(!output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text.starts_with("Location: "))),"Map tooltip remained after leaving the map");
            for pressed in [true, false] {
                let _ = ctx.run(
                    egui::RawInput {
                        events: vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                        ..input()
                    },
                    |ctx| app.render(ctx),
                );
            }
            assert_eq!(app.selected, Some(5));
            assert!(app.expanded.contains(&4));
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..input()
                },
                |ctx| app.render(ctx),
            );
            assert_eq!(app.current, 4);
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..input()
                },
                |ctx| app.render(ctx),
            );
            assert_eq!(app.current, 3);
        }
    }
    fn frame_input(size: egui::Vec2, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        }
    }
    /// Renders until layout, caches and any measuring passes have settled.
    fn settle(ctx: &egui::Context, app: &mut Canopy, size: egui::Vec2) -> egui::FullOutput {
        let mut output = ctx.run(frame_input(size, vec![]), |ctx| app.render(ctx));
        for _ in 0..3 {
            output = ctx.run(frame_input(size, vec![]), |ctx| app.render(ctx));
        }
        output
    }
    fn texts(output: &egui::FullOutput) -> Vec<(String, Rect, usize)> {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some((
                    t.galley.job.text.clone(),
                    t.visual_bounding_rect(),
                    t.galley.rows.len(),
                )),
                _ => None,
            })
            .collect()
    }
    fn click_label(ctx: &egui::Context, app: &mut Canopy, size: egui::Vec2, label: &str) {
        let output = settle(ctx, app, size);
        let point = texts(&output)
            .into_iter()
            .find(|(text, _, _)| text == label)
            .unwrap_or_else(|| panic!("Missing control {label}"))
            .1
            .center();
        for pressed in [true, false] {
            let _ = ctx.run(
                frame_input(
                    size,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                ),
                |ctx| app.render(ctx),
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn refreshed_exfat_drive_can_be_selected_without_starting_a_scan() {
        for size in [vec2(1050.0, 700.0), vec2(1400.0, 900.0)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut app = app(None);
            let (sender, receiver) = mpsc::channel();
            app.drive_receiver = Some(receiver);
            sender
                .send(vec![scanner::windows::Drive {
                    root: "E:\\".into(),
                    label: "External fixture".into(),
                    filesystem: "exFAT".into(),
                    total: 100_000,
                    free: 20_000,
                    serial: 123,
                }])
                .unwrap();
            app.poll();
            assert!(app.drive_receiver.is_none());
            assert_eq!(app.drives.len(), 1);
            click_label(&ctx, &mut app, size, "Choose drive");
            click_label(&ctx, &mut app, size, "E:\\ exFAT");
            assert_eq!(app.path, "E:\\");
            assert!(app.scan.is_none() && app.receiver.is_none());
            let output = settle(&ctx, &mut app, size);
            assert!(texts(&output).iter().any(|(text, _, _)| text == "exFAT"));
        }
    }
    fn center_of(app: &Canopy, id: usize) -> egui::Pos2 {
        tile_rect(app.map_cache.iter().find(|t| t.id == id).unwrap()).center()
    }
    #[test]
    fn search_typing_and_clear_preserve_focus_and_toolbar_positions() {
        for size in [vec2(1050.0, 700.0), vec2(1400.0, 900.0)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut app = app(Some(fixture()));
            let controls = |output: &egui::FullOutput| {
                ["Rescan", "Open / save", "Recycle Bin"].map(|label| {
                    texts(output)
                        .into_iter()
                        .find(|(text, _, _)| text == label)
                        .unwrap()
                        .1
                })
            };
            let empty = settle(&ctx, &mut app, size);
            let original = controls(&empty);
            let search_id = egui::Id::new("global-search");
            click_label(&ctx, &mut app, size, "Ctrl+K");
            assert!(ctx.memory(|memory| memory.has_focus(search_id)));
            ctx.memory_mut(|memory| memory.surrender_focus(search_id));
            let _ = ctx.run(
                frame_input(
                    size,
                    vec![egui::Event::Key {
                        key: egui::Key::K,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::CTRL,
                    }],
                ),
                |ctx| app.render(ctx),
            );
            assert!(ctx.memory(|memory| memory.has_focus(search_id)));
            let query = format!("ext:mp4 >1GB older:1y {}", "long-file-name-".repeat(80));
            let _ = ctx.run(
                frame_input(size, vec![egui::Event::Text(query.clone())]),
                |ctx| app.render(ctx),
            );
            assert_eq!(app.query, query);
            let typed = settle(&ctx, &mut app, size);
            for (before, after) in original.into_iter().zip(controls(&typed)) {
                assert!((before.left() - after.left()).abs() < 1.0);
                assert!((before.right() - after.right()).abs() < 1.0);
                assert!(after.right() <= size.x);
            }
            click_label(&ctx, &mut app, size, "×");
            assert!(app.query.is_empty());
            assert!(
                ctx.memory(|memory| memory.has_focus(search_id)),
                "Clearing must let typing continue immediately"
            );
            let cleared = settle(&ctx, &mut app, size);
            assert_eq!(controls(&cleared), original);
            let _ = ctx.run(
                frame_input(size, vec![egui::Event::Text("archive".into())]),
                |ctx| app.render(ctx),
            );
            assert_eq!(app.query, "archive");
        }
    }

    #[test]
    fn breadcrumbs_are_whole_and_separate_from_size_controls() {
        for width in [760.0, 1100.0] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut scan = fixture();
            scan.nodes[4].name = "StudioProjects".into();
            let mut app = app(Some(scan));
            app.current = 4;
            let scan = app.scan.clone().unwrap();
            let mut output = None;
            for _ in 0..3 {
                output = Some(ctx.run(frame_input(vec2(width, 160.0), vec![]), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.actions(ui, &scan));
                }));
            }
            let output = output.unwrap();
            let mut navigation = Rect::NOTHING;
            for name in ["C:\\", "Users", "Guest", "Desktop", "StudioProjects"] {
                let shape = output.shapes.iter().find(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == name)
                }).unwrap_or_else(|| panic!("Missing breadcrumb {name:?} at {width}"));
                let egui::Shape::Text(text) = &shape.shape else {
                    unreachable!()
                };
                let bounds = text.visual_bounding_rect();
                assert!(
                    shape.clip_rect.expand(1.0).contains_rect(bounds),
                    "Partially clipped breadcrumb {name:?}: {bounds:?} inside {:?}",
                    shape.clip_rect
                );
                navigation = navigation.union(bounds);
            }
            for (text, bounds, rows) in texts(&output) {
                if text.starts_with("Size on disk")
                    || text.starts_with("File size")
                    || text.starts_with("Folder size on disk")
                    || text.starts_with("Folder file size")
                    || text == "Map options"
                {
                    assert_eq!(rows, 1, "Wrapped control {text:?}");
                    assert!(
                        bounds.top() > navigation.bottom(),
                        "Control {text:?} crowded the breadcrumb row"
                    );
                    assert!(bounds.right() <= width, "Control {text:?} overflowed");
                }
            }
        }
    }
    #[test]
    fn simplified_controls_change_measure_and_reset_map_options() {
        let size = vec2(1050.0, 700.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        let output = settle(&ctx, &mut app, size);
        let labels: Vec<_> = texts(&output)
            .into_iter()
            .map(|(text, _, _)| text)
            .collect();
        assert!(labels.iter().any(|text| text == "Map options"));
        assert!(!labels.iter().any(|text| text.starts_with("Depth:")
            || text.starts_with("Min:")
            || text.starts_with("File size")));
        click_label(&ctx, &mut app, size, "Size on disk · 80.53 GB");
        click_label(&ctx, &mut app, size, "File size · 83.75 GB");
        assert_eq!(app.metric, Metric::Logical);
        settle(&ctx, &mut app, size);
        assert_eq!(
            app.map_cache.iter().find(|tile| tile.id == 5).unwrap().size,
            app.scan.as_ref().unwrap().nodes[5].logical
        );
        // Folder totals and metric choices follow the opened scope. Returning
        // to the drive restores its label without changing the size measure.
        app.navigate(4);
        click_label(&ctx, &mut app, size, "Folder file size · 71.94 GB");
        click_label(&ctx, &mut app, size, "Folder size on disk · 69.79 GB");
        assert_eq!(app.metric, Metric::Allocated);
        settle(&ctx, &mut app, size);
        assert_eq!(
            app.map_cache.iter().find(|tile| tile.id == 5).unwrap().size,
            40 << 30
        );
        assert!(!app.map_cache.iter().any(|tile| tile.id == 7));
        click_label(&ctx, &mut app, size, "Folder size on disk · 69.79 GB");
        click_label(&ctx, &mut app, size, "Folder file size · 71.94 GB");
        assert_eq!(app.metric, Metric::Logical);
        app.navigate(0);
        let drive = settle(&ctx, &mut app, size);
        assert!(
            texts(&drive)
                .iter()
                .any(|(text, _, _)| text == "File size · 83.75 GB")
        );
        click_label(&ctx, &mut app, size, "Map options");
        click_label(&ctx, &mut app, size, "2");
        assert_eq!(app.depth, 2);
        let custom = settle(&ctx, &mut app, size);
        assert!(
            texts(&custom)
                .iter()
                .any(|(text, _, _)| text == "Map options · Custom")
        );
        // Options stay open so depth and grouping can be set together.
        click_label(&ctx, &mut app, size, "1.00 MB");
        assert_eq!(app.minimum, 1_000_000);
        click_label(&ctx, &mut app, size, "Reset map options");
        assert_eq!((app.depth, app.minimum), (96, 0));
        settle(&ctx, &mut app, size);
        assert!(
            app.map_cache
                .iter()
                .any(|tile| tile.id == 5 && tile.kind == treemap::Kind::File)
        );
        assert_eq!(app.metric, Metric::Logical);
    }
    #[test]
    fn breadcrumb_overflow_menu_keeps_hidden_ancestors_navigable() {
        for width in [760.0, 1100.0] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut scan = fixture();
            for node in &mut scan.nodes[1..4] {
                node.name = format!("{} with a long folder name ", node.name).repeat(4);
            }
            scan.nodes[4].name = "StudioProjects".into();
            let ancestor = scan.nodes[1].name.clone();
            let mut app = app(Some(scan));
            app.current = 4;
            let scan = app.scan.clone().unwrap();
            let render = |app: &mut Canopy, events| {
                ctx.run(frame_input(vec2(width, 240.0), events), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.actions(ui, &scan));
                })
            };
            let mut output = render(&mut app, vec![]);
            for _ in 0..3 {
                output = render(&mut app, vec![]);
            }
            let bounds = |output: &egui::FullOutput, label: &str| {
                texts(output)
                    .into_iter()
                    .find(|(text, _, _)| text == label)
                    .unwrap_or_else(|| panic!("Missing breadcrumb/menu item {label:?} at {width}"))
                    .1
            };
            let current = bounds(&output, "StudioProjects");
            assert!(current.right() <= width && current.left() >= 0.0);
            let overflow = bounds(&output, "…").center();
            let click_events = |point, pressed| {
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            for pressed in [true, false] {
                output = render(&mut app, click_events(overflow, pressed));
            }
            for _ in 0..2 {
                output = render(&mut app, vec![]);
            }
            let item = bounds(&output, &ancestor);
            assert!(
                item.right() <= width,
                "Overflow menu escaped the window: {item:?}"
            );
            for pressed in [true, false] {
                let _ = render(&mut app, click_events(item.center(), pressed));
            }
            assert_eq!(app.current, 1, "Hidden ancestor could not be opened");
        }
    }
    #[test]
    fn tooltip_lines_never_wrap_however_the_pointer_moves() {
        // egui's Area caps each frame's width at the width it measured on the
        // previous frame. With one shared id, passing over a narrow tooltip
        // squeezed every later one: "Location: C:\...\Projects" broke into
        // three rows 151 px wide. Sweep the pointer across the whole map,
        // one frame per step, and require every tooltip line to stay whole.
        for size in [vec2(1050.0, 700.0), vec2(1400.0, 900.0)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut app = app(Some(fixture()));
            settle(&ctx, &mut app, size);
            let map = app
                .map_cache
                .iter()
                .map(tile_rect)
                .fold(Rect::NOTHING, |all, tile| all.union(tile));
            let mut seen = 0;
            let (mut y, mut direction) = (map.top() + 3.0, 1.0_f32);
            while y < map.bottom() + 20.0 {
                let mut x = if direction > 0.0 {
                    map.left() + 3.0
                } else {
                    map.right() + 40.0
                };
                while x > map.left() - 10.0 && x < map.right() + 60.0 {
                    let point = egui::pos2(x, y);
                    let output = ctx.run(
                        frame_input(size, vec![egui::Event::PointerMoved(point)]),
                        |ctx| app.render(ctx),
                    );
                    for (text, bounds, rows) in texts(&output) {
                        let tooltip_line = text.starts_with("Location: ")
                            || text.starts_with("File size: ")
                            || text.starts_with("Size on disk: ")
                            || text.contains("of indexed drive");
                        if tooltip_line {
                            seen += 1;
                            assert_eq!(
                                rows,
                                1,
                                "{text:?} wrapped to {rows} rows ({}px wide) with the pointer at {point:?} in a {size:?} window",
                                bounds.width()
                            );
                        }
                    }
                    x += 25.0 * direction;
                }
                direction = -direction;
                y += 40.0;
            }
            assert!(seen > 100, "the sweep barely hit any tooltip ({seen})");
        }
    }
    #[test]
    fn tooltip_opens_away_from_the_screen_edge() {
        let size = vec2(1050.0, 700.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        settle(&ctx, &mut app, size);
        // Hovering near the map's right edge: a tooltip opened to the right of
        // the pointer would run out of the window.
        let root = tile_rect(app.map_cache.iter().find(|t| t.id == 0).unwrap());
        let point = egui::pos2(root.right() - 20.0, root.center().y);
        assert!(
            point.x + 14.0 + 360.0 > size.x,
            "test point is not near the edge"
        );
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run(
                frame_input(size, vec![egui::Event::PointerMoved(point)]),
                |ctx| app.render(ctx),
            ));
        }
        let tooltip = texts(&output.unwrap())
            .into_iter()
            .filter(|(text, _, _)| {
                text.starts_with("Location: ")
                    || text.starts_with("File size: ")
                    || text.starts_with("Size on disk: ")
            })
            .fold(Rect::NOTHING, |all, (_, bounds, _)| all.union(bounds));
        assert!(tooltip.is_positive(), "no tooltip near the edge");
        assert!(
            tooltip.right() <= point.x && !tooltip.contains(point),
            "tooltip {tooltip:?} covers the pointer {point:?}"
        );
        assert!(
            tooltip.left() >= 0.0 && tooltip.right() <= size.x,
            "tooltip clipped"
        );
    }
    #[test]
    fn hovering_keeps_the_map_visible_with_tooltip_and_tile_outline() {
        let size = vec2(1400.0, 900.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        settle(&ctx, &mut app, size);
        let dim = Color32::from_black_alpha(110);
        let dimmed = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == dim))
                .count()
        };
        let hover = |app: &mut Canopy, id: usize| {
            let point = center_of(app, id);
            let mut output = None;
            for _ in 0..2 {
                output = Some(ctx.run(
                    frame_input(size, vec![egui::Event::PointerMoved(point)]),
                    |ctx| app.render(ctx),
                ));
            }
            let output = output.unwrap();
            assert!(
                texts(&output)
                    .iter()
                    .any(|(text, _, _)| text == &app.scan.as_ref().unwrap().nodes[id].name),
                "Missing hover tooltip"
            );
            let tile = tile_rect(app.map_cache.iter().find(|tile| tile.id == id).unwrap());
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(r) if r.rect == tile && r.stroke == Stroke::new(1.0_f32, TEXT))), "Missing hovered tile outline");
            dimmed(&output)
        };
        // capture.mp4 sits in `Projects`, which fills most but not all of the map.
        assert_eq!(hover(&mut app, 5), 0, "Hover darkened the surrounding map");
        // Root files retain the same non-dimming hover behaviour.
        assert_eq!(hover(&mut app, 7), 0);
        assert_eq!(dimmed(&settle(&ctx, &mut app, size)), 0);
    }
    #[test]
    fn thin_remainder_strip_has_no_letter_by_letter_label() {
        // The "Outside indexed files" block is a thin strip next to a full-drive
        // map. Its label used to wrap one character per line inside it.
        for size in [vec2(1050.0, 700.0), vec2(1400.0, 900.0)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut app = app(Some(fixture()));
            let output = settle(&ctx, &mut app, size);
            for (text, bounds, rows) in texts(&output) {
                if text.starts_with("Outside indexed files") {
                    assert!(
                        bounds.width() >= 60.0 && rows <= 4,
                        "cramped remainder label {bounds:?} in {rows} rows at {size:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn folder_names_stay_in_reserved_headers_at_every_readable_depth() {
        for size in [vec2(1050.0, 700.0), vec2(1400.0, 900.0)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut app = app(Some(fixture()));
            let output = settle(&ctx, &mut app, size);
            // The sidebar tree repeats every folder name, so read only the map.
            let map = app
                .map_cache
                .iter()
                .map(tile_rect)
                .fold(Rect::NOTHING, |all, tile| all.union(tile));
            let drawn: Vec<_> = texts(&output)
                .into_iter()
                .filter(|(_, bounds, _)| map.contains_rect(*bounds))
                .map(|(t, _, _)| t)
                .collect();
            for (name, size_label) in [
                ("Users", " · 69.79 GB"),
                ("Guest", " · 69.79 GB"),
                ("Desktop", " · 69.79 GB"),
                ("Projects", " · 69.79 GB"),
            ] {
                assert!(
                    drawn.iter().any(|t| t == name),
                    "no label for {name} at {size:?}"
                );
                assert!(
                    drawn.iter().any(|t| t == size_label),
                    "no size beside {name} at {size:?}"
                );
            }
            for tile in app.map_cache.iter().filter(|tile| tile.header) {
                let node = &app.scan.as_ref().unwrap().nodes[tile.id];
                let heading = Rect::from_min_size(
                    tile_rect(tile).min,
                    vec2(tile.rect.w, treemap::HEADER_HEIGHT),
                );
                let title = texts(&output)
                    .into_iter()
                    .find(|(text, bounds, _)| text == &node.name && heading.contains_rect(*bounds));
                assert!(
                    title.is_some(),
                    "{} escaped its header at {size:?}",
                    node.name
                );
            }
        }
    }
    #[test]
    fn crowded_file_names_stay_in_hover_details_instead_of_becoming_prefixes() {
        let mut scan = fixture();
        let long_name = format!("textures_{}.bundle", "environment_asset_".repeat(20));
        scan.nodes[5].name = long_name.clone();
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(scan));
        let size = vec2(1400.0, 900.0);
        let output = settle(&ctx, &mut app, size);
        let tile = app.map_cache.iter().find(|tile| tile.id == 5).unwrap();
        let block = tile_rect(tile);
        assert!(
            !texts(&output)
                .iter()
                .any(|(text, bounds, _)| block.contains_rect(*bounds)
                    && text.starts_with("textures_"))
        );
        let output = ctx.run(
            frame_input(size, vec![egui::Event::PointerMoved(block.center())]),
            |ctx| app.render(ctx),
        );
        assert!(texts(&output).iter().any(|(text, _, _)| text == &long_name));
    }
    #[test]
    fn folder_label_gives_up_the_name_before_the_size() {
        let ctx = egui::Context::default();
        // Fonts exist only after a first pass.
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let painter =
            egui::Painter::new(ctx.clone(), egui::LayerId::background(), Rect::EVERYTHING);
        let font = FontId::proportional(13.0);
        let wide = name_and_size(&painter, "Windows", "23.90 GB", 400.0, font.clone(), TEXT);
        assert_eq!(wide.len(), 2);
        assert_eq!(wide[0].1.job.text, "Windows");
        let narrow = name_and_size(
            &painter,
            "Program Files (x86)",
            "214.17 GB",
            130.0,
            font.clone(),
            TEXT,
        );
        assert_eq!(narrow.len(), 2, "name should shorten, size must stay");
        assert!(
            narrow[0].1.elided,
            "the name should be shortened with an ellipsis"
        );
        assert_eq!(narrow[1].1.job.text, " · 214.17 GB");
        let width: f32 = narrow
            .iter()
            .map(|(x, g)| x + g.size().x)
            .fold(0.0, f32::max);
        assert!(
            width <= 131.0,
            "label {width}px is wider than its 130px header"
        );
        let tiny = name_and_size(
            &painter,
            "Program Files (x86)",
            "214.17 GB",
            60.0,
            font,
            TEXT,
        );
        // Too narrow to fit a name and a size: keep the name, never a bare number.
        assert_eq!(tiny.len(), 1);
        assert_eq!(tiny[0].1.job.text, "Program Files (x86)");
        assert!(tiny[0].1.elided && tiny[0].1.size().x <= 61.0);
    }
    #[test]
    fn tree_says_how_many_children_it_hides() {
        let mut scan = Scan {
            root: "C:\\".into(),
            stats: ScanStats::default(),
            nodes: vec![Node::new("C:\\".into(), None, true)],
        };
        for i in 0..300 {
            let mut node = Node::new(format!("file-{i:03}.dat"), Some(0), false);
            node.allocated = 1000 + i as u64;
            node.logical = node.allocated;
            scan.nodes.push(node);
            scan.nodes[0].children.push(i + 1);
        }
        scan.finish();
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(scan));
        app.selected = None;
        settle(&ctx, &mut app, vec2(1400.0, 900.0));
        let more: Vec<_> = app
            .tree_rows
            .iter()
            .filter_map(|row| match row {
                TreeRow::More { parent, hidden, .. } => Some((*parent, *hidden)),
                _ => None,
            })
            .collect();
        assert_eq!(more, vec![(0, 300 - TREE_FANOUT)]);
        let entries = app
            .tree_rows
            .iter()
            .filter(|row| matches!(row, TreeRow::Entry { .. }))
            .count();
        assert_eq!(entries, TREE_FANOUT);
        assert!(matches!(app.tree_rows.last(), Some(TreeRow::More { .. })));
    }
    #[test]
    fn deep_tree_indentation_is_bounded_and_monotonic() {
        let mut previous = -1.0;
        for depth in 0..40 {
            let indent = tree_indent(depth);
            assert!(indent >= previous, "indent shrank at depth {depth}");
            previous = indent;
        }
        assert!(tree_indent(40) <= 12.0 * 6.0 + 4.0 * 8.0);
        assert!(
            tree_indent(7) > tree_indent(6),
            "deep levels must stay distinguishable"
        );
    }
    #[test]
    fn zooming_into_a_folder_keeps_the_search_and_type_filter() {
        let mut app = app(Some(fixture()));
        app.query = "ext:mp4 >1GB".into();
        app.category = Some(Category::Media);
        app.navigate(4);
        assert_eq!(app.current, 4);
        assert_eq!(app.query, "ext:mp4 >1GB");
        assert_eq!(app.category, Some(Category::Media));
    }
    #[test]
    fn sidebar_follows_scope_and_zero_entries_remain_accessible() {
        let size = vec2(1050.0, 700.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut scan = fixture();
        scan.nodes
            .push(Node::new("empty.dat".into(), Some(4), false));
        let mut resident = Node::new("resident.txt".into(), Some(4), false);
        resident.logical = 123;
        scan.nodes.push(resident);
        scan.nodes[4].children.extend([8, 9]);
        scan.finish();
        let mut app = app(Some(scan));
        app.navigate(4);
        let output = settle(&ctx, &mut app, size);
        let ids = |app: &Canopy| {
            app.tree_rows
                .iter()
                .filter_map(|row| match row {
                    TreeRow::Entry { id, .. } => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&app), vec![5, 6]);
        assert!(
            texts(&output)
                .iter()
                .any(|(text, rect, _)| text == "Projects" && rect.left() < 238.0),
            "Scope name must stay visible in the sidebar"
        );
        click_label(&ctx, &mut app, size, "Options");
        click_label(&ctx, &mut app, size, "Show zero-size entries");
        assert!(app.show_zero_entries);
        settle(&ctx, &mut app, size);
        assert!(ids(&app).contains(&8) && ids(&app).contains(&9));
        // Explicit selection from Files keeps a zero-size item reachable even
        // when the default sidebar filter is restored.
        app.show_zero_entries = false;
        app.tree_dirty = true;
        app.select(8);
        settle(&ctx, &mut app, size);
        assert!(ids(&app).contains(&8));
        assert!(!ids(&app).contains(&9));
        app.select(4);
        app.metric = Metric::Logical;
        app.tree_dirty = true;
        settle(&ctx, &mut app, size);
        assert_eq!(ids(&app), vec![5, 6, 9]);
        app.navigate(3);
        settle(&ctx, &mut app, size);
        assert!(
            !ids(&app).contains(&7),
            "Drive siblings leaked into a folder scope"
        );
    }
    #[test]
    fn compact_color_key_fits_and_clicks_preserve_search() {
        let size = vec2(574.0, 100.0); // Narrowest workspace with a 440-point sidebar.
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        app.query = "capture".into();
        let scan = app.scan.clone().unwrap();
        let render = |app: &mut Canopy, events| {
            ctx.run(frame_input(size, events), |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show(ctx, |ui| app.breakdown(ui, &scan));
            })
        };
        let mut output = render(&mut app, vec![]);
        for _ in 0..3 {
            output = render(&mut app, vec![]);
        }
        let labels = [
            "Media",
            "Archives",
            "Docs",
            "Code",
            "Apps",
            "Assets",
            "System",
            "Other",
            "All types",
        ];
        let mut previous_right = 0.0;
        let mut centers = vec![];
        for label in labels {
            let bounds = texts(&output)
                .into_iter()
                .find(|(text, _, _)| text == label)
                .unwrap()
                .1;
            assert!(
                bounds.left() >= previous_right
                    && bounds.right() <= size.x
                    && bounds.bottom() <= 36.0,
                "Clipped or overlapping filter {label}: {bounds:?}"
            );
            previous_right = bounds.right();
            centers.push(bounds.center());
        }
        for (index, category) in Category::ALL.iter().enumerate() {
            for point in [centers[index], centers[8]] {
                for pressed in [true, false] {
                    let _ = render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(point),
                            egui::Event::PointerButton {
                                pos: point,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                assert_eq!(
                    app.category,
                    if point == centers[index] {
                        Some(*category)
                    } else {
                        None
                    }
                );
                assert_eq!(app.query, "capture");
            }
        }
    }
    #[test]
    fn right_click_on_a_tile_offers_actions_for_that_tile() {
        let size = vec2(1400.0, 900.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        settle(&ctx, &mut app, size);
        let point = center_of(&app, 5);
        let press = |pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(point), press(true)],
            vec![press(false)],
            vec![],
            vec![],
            vec![],
        ] {
            let _ = ctx.run(frame_input(size, events), |ctx| app.render(ctx));
        }
        assert_eq!(app.map_menu, Some(5));
        assert_eq!(app.selected, Some(5));
        let output = ctx.run(frame_input(size, vec![]), |ctx| app.render(ctx));
        let drawn: Vec<_> = texts(&output).into_iter().map(|(t, _, _)| t).collect();
        for item in [
            "Only show Media",
            "Show in Explorer",
            "Copy path",
            "Delete…",
        ] {
            assert!(drawn.iter().any(|t| t == item), "menu lacks {item}");
        }
        assert!(
            !drawn.iter().any(|t| t.starts_with("Location: ")),
            "tooltip covers the open menu"
        );
        // Choosing it isolates the type everywhere.
        let target = texts(&output)
            .into_iter()
            .find(|(t, _, _)| t == "Only show Media")
            .unwrap()
            .1
            .center();
        for pressed in [true, false] {
            let _ = ctx.run(
                frame_input(
                    size,
                    vec![
                        egui::Event::PointerMoved(target),
                        egui::Event::PointerButton {
                            pos: target,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                ),
                |ctx| app.render(ctx),
            );
        }
        assert_eq!(app.category, Some(Category::Media));
    }
    #[test]
    fn delete_confirmation_shows_exact_target_and_cancel_does_not_start_a_worker() {
        for size in [vec2(1050.0, 700.0), vec2(1400.0, 900.0)] {
            for id in [4, 5] {
                let ctx = egui::Context::default();
                configure_ui(&ctx);
                let mut app = app(Some(fixture()));
                app.delete_pending = Some(DeleteConfirmation::new(
                    recycle::Target::from_scan(app.scan.as_ref().unwrap(), id).unwrap(),
                    recycle::Method::RecycleBin,
                ));
                let expected = app
                    .delete_pending
                    .as_ref()
                    .unwrap()
                    .target
                    .path
                    .display()
                    .to_string();
                let output = settle(&ctx, &mut app, size);
                let found = texts(&output);
                assert!(
                    found.iter().any(|(text, _, _)| text == &expected),
                    "Confirmation omitted the full target path"
                );
                for label in [
                    if id == 4 {
                        "Delete this folder?"
                    } else {
                        "Delete this file?"
                    },
                    "Move to Recycle Bin",
                    "Cancel",
                ] {
                    let bounds = found.iter().find(|(text, _, _)| text == label).unwrap().1;
                    assert!(
                        ctx.content_rect().contains_rect(bounds),
                        "Confirmation control clipped: {label}"
                    );
                }
                let cancel = found
                    .iter()
                    .find(|(text, _, _)| text == "Cancel")
                    .unwrap()
                    .1
                    .center();
                for pressed in [true, false] {
                    let _ = ctx.run(
                        frame_input(
                            size,
                            vec![
                                egui::Event::PointerMoved(cancel),
                                egui::Event::PointerButton {
                                    pos: cancel,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                        ),
                        |ctx| app.render(ctx),
                    );
                }
                assert!(app.delete_pending.is_none());
                assert!(app.deletion.is_none());
                assert!(!app.scan_stale);
            }
        }
    }
    #[test]
    fn delete_is_disabled_for_roots_grouped_tiles_and_stale_results() {
        for (id, grouped, stale) in [(0, false, false), (4, true, false), (5, false, true)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut app = app(Some(fixture()));
            app.scan_stale = stale;
            let scan = app.scan.clone().unwrap();
            let render = |app: &mut Canopy, events| {
                ctx.run(frame_input(vec2(600.0, 200.0), events), |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| app.delete_menu_item(ui, &scan, id, grouped));
                })
            };
            let output = render(&mut app, vec![]);
            let point = texts(&output)
                .into_iter()
                .find(|(text, _, _)| text == "Delete…")
                .unwrap()
                .1
                .center();
            for pressed in [true, false] {
                let _ = render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(app.delete_pending.is_none());
            assert!(app.deletion.is_none());
        }
    }
    fn completed_delete(
        app: &mut Canopy,
        method: recycle::Method,
        result: Result<recycle::Outcome, String>,
    ) {
        let target = recycle::Target::from_scan(app.scan.as_ref().unwrap(), 5).unwrap();
        let (sender, receiver) = mpsc::channel();
        app.deletion = Some(DeleteJob {
            generation: app.generation,
            target,
            method,
            receiver,
        });
        sender.send(result).unwrap();
        app.poll();
    }
    #[test]
    fn delete_completion_marks_results_stale_without_inventing_freed_bytes() {
        for method in [recycle::Method::RecycleBin, recycle::Method::Permanent] {
            let mut app = app(Some(fixture()));
            let before = app.scan.as_ref().unwrap().nodes[0].allocated;
            completed_delete(&mut app, method, Ok(recycle::Outcome::Deleted));
            assert!(app.scan_stale);
            let wording = match method {
                recycle::Method::RecycleBin => "Recycle Bin",
                recycle::Method::Permanent => "Permanently deleted",
            };
            assert!(app.message.contains(wording) && app.message.contains("Rescan"));
            assert_eq!(app.scan.as_ref().unwrap().nodes[0].allocated, before);
            assert!(app.deletion.is_none());
            assert!(app.delete_pending.is_none());
            Arc::get_mut(app.scan.as_mut().unwrap())
                .unwrap()
                .stats
                .runtime = crate::model::RuntimeState::new(crate::model::Provenance::Live);
            completed_delete(
                &mut app,
                method,
                Err("Windows could not delete this item".into()),
            );
            assert!(
                app.scan_error
                    .as_ref()
                    .unwrap()
                    .contains("could not delete")
            );
            assert!(
                app.delete_pending.is_none(),
                "An ordinary error must not offer permanent deletion"
            );
            assert_eq!(app.scan.as_ref().unwrap().nodes[0].allocated, before);
        }
    }
    #[test]
    fn recycle_refusal_opens_confirmation_without_starting_permanent_deletion() {
        let mut app = app(Some(fixture()));
        let expected = recycle::Target::from_scan(app.scan.as_ref().unwrap(), 5).unwrap();
        completed_delete(
            &mut app,
            recycle::Method::RecycleBin,
            Ok(recycle::Outcome::CannotRecycle),
        );
        let pending = app.delete_pending.as_ref().unwrap();
        assert_eq!(pending.target.path, expected.path);
        assert_eq!(pending.target.name, expected.name);
        assert_eq!(pending.target.logical, expected.logical);
        assert_eq!(pending.method, recycle::Method::Permanent);
        assert!(pending.focus_cancel);
        assert!(app.deletion.is_none());
        assert!(!app.scan_stale);
        assert!(app.scan_error.is_none());
    }
    #[test]
    fn permanent_confirmation_shows_exact_scope_and_cancel_starts_no_worker() {
        for size in [vec2(1050.0, 700.0), vec2(1400.0, 900.0)] {
            for id in [4, 5] {
                for cancel_key in [None, Some(egui::Key::Escape), Some(egui::Key::Enter)] {
                    let ctx = egui::Context::default();
                    configure_ui(&ctx);
                    let mut app = app(Some(fixture()));
                    let target =
                        recycle::Target::from_scan(app.scan.as_ref().unwrap(), id).unwrap();
                    let expected = target.path.display().to_string();
                    app.delete_pending =
                        Some(DeleteConfirmation::new(target, recycle::Method::Permanent));
                    app.scan_stale = true;
                    let output = settle(&ctx, &mut app, size);
                    let found = texts(&output);
                    for label in [
                        expected.as_str(),
                        if id == 4 {
                            "Permanently delete this folder?"
                        } else {
                            "Permanently delete this file?"
                        },
                        "Windows cannot recycle this item.",
                        "This cannot be undone. You cannot restore it from the Recycle Bin.",
                        "Delete permanently",
                        "Cancel",
                    ] {
                        let bounds = found.iter().find(|(text, _, _)| text == label).unwrap().1;
                        assert!(
                            ctx.content_rect().contains_rect(bounds),
                            "Permanent confirmation clipped: {label}"
                        );
                    }
                    if id == 4 {
                        assert!(
                            found
                                .iter()
                                .any(|(text, _, _)| text.contains("all its current contents"))
                        );
                    }
                    assert!(
                        !found
                            .iter()
                            .any(|(text, _, _)| text == "Move to Recycle Bin")
                    );
                    if let Some(key) = cancel_key {
                        let _ = ctx.run(
                            frame_input(
                                size,
                                vec![egui::Event::Key {
                                    key,
                                    physical_key: None,
                                    pressed: true,
                                    repeat: false,
                                    modifiers: egui::Modifiers::NONE,
                                }],
                            ),
                            |ctx| app.render(ctx),
                        );
                    } else {
                        click_label(&ctx, &mut app, size, "Cancel");
                    }
                    assert!(
                        app.delete_pending.is_none(),
                        "Cancel/default Enter/Escape must close the prompt"
                    );
                    assert!(app.deletion.is_none());
                    assert!(app.scan_stale);
                }
            }
        }
    }
    #[cfg(windows)]
    #[test]
    fn permanent_confirmation_revalidates_changed_file_before_operating() {
        use canopy::model::{Node, ScanStats};
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(workspace.join("artifacts")).unwrap();
        let fixture = tempfile::Builder::new()
            .prefix("canopy-confirm-guard-")
            .tempdir_in(workspace.join("artifacts"))
            .unwrap();
        assert!(
            fixture
                .path()
                .canonicalize()
                .unwrap()
                .starts_with(workspace.canonicalize().unwrap())
        );
        let path = fixture.path().join("new-disposable-file.txt");
        std::fs::write(&path, b"before").unwrap();
        let mut scan = Scan {
            root: fixture.path().into(),
            nodes: vec![
                Node::new("fixture".into(), None, true),
                Node::new("new-disposable-file.txt".into(), Some(0), false),
            ],
            stats: ScanStats::default(),
        };
        scan.stats.runtime = crate::model::RuntimeState::new(crate::model::Provenance::Live);
        for id in 0..scan.nodes.len() {
            let handle = scanner::windows::identity_handle(&scan.path(id), true).unwrap();
            let (volume, reference) = scanner::windows::handle_identity(&handle).unwrap();
            scan.stats.directory_streams_captured = true;
            scan.stats.volume_serial = Some(volume);
            scan.nodes[id].reference = Some(reference);
        }
        scan.nodes[1].logical = 6;
        scan.nodes[0].children = vec![1];
        scan.finish();
        let target = recycle::Target::from_scan(&scan, 1).unwrap();
        let mut app = app(Some(scan));
        app.selected = Some(1);
        app.delete_pending = Some(DeleteConfirmation::new(target, recycle::Method::Permanent));
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let size = vec2(1050.0, 700.0);
        settle(&ctx, &mut app, size);
        std::fs::write(&path, b"changed while confirmation was open").unwrap();
        click_label(&ctx, &mut app, size, "Delete permanently");
        assert!(app.delete_pending.is_none());
        assert_eq!(
            app.deletion.as_ref().unwrap().method,
            recycle::Method::Permanent
        );
        assert!(app.scan_stale);
        for _ in 0..100 {
            app.poll();
            if app.deletion.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(app.deletion.is_none());
        assert!(app.scan_error.as_ref().unwrap().contains("size changed"));
        assert!(
            app.delete_pending.is_none(),
            "Validation errors must not re-offer permanent deletion"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"changed while confirmation was open"
        );
    }
    #[test]
    fn disconnected_delete_worker_never_offers_permanent_deletion() {
        let mut app = app(Some(fixture()));
        let target = recycle::Target::from_scan(app.scan.as_ref().unwrap(), 5).unwrap();
        let (sender, receiver) = mpsc::channel();
        app.deletion = Some(DeleteJob {
            generation: app.generation,
            target,
            method: recycle::Method::RecycleBin,
            receiver,
        });
        drop(sender);
        app.poll();
        assert!(app.scan_stale);
        assert!(app.scan_error.as_ref().unwrap().contains("disconnected"));
        assert!(app.delete_pending.is_none());
        assert!(app.deletion.is_none());
    }
    #[test]
    fn right_click_on_empty_space_opens_no_menu() {
        let size = vec2(1400.0, 900.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        settle(&ctx, &mut app, size);
        // The map is full to its right edge, so aim at the empty message area
        // of a filtered map instead: nothing matches, so no tile is under it.
        app.query = "ext:nothing".into();
        // The filter runs on a worker thread; wait for its answer.
        for _ in 0..500 {
            let _ = ctx.run(frame_input(size, vec![]), |ctx| app.render(ctx));
            if app.weights.is_some() && app.map_cache.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(
            app.map_cache.is_empty(),
            "filter never produced an empty map"
        );
        let point = egui::pos2(700.0, 300.0);
        for pressed in [true, false] {
            let _ = ctx.run(
                frame_input(
                    size,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                ),
                |ctx| app.render(ctx),
            );
        }
        settle(&ctx, &mut app, size);
        assert_eq!(app.map_menu, None);
        assert!(
            !egui::Popup::is_any_open(&ctx),
            "an empty menu was left open"
        );
    }
    #[test]
    fn files_sort_by_modified_date_and_show_it() {
        let mut scan = fixture();
        scan.nodes[5].modified = 1_790_812_800; // 2026-10-01
        scan.nodes[6].modified = 1_600_000_000; // 2020-09-13
        scan.nodes[7].modified = 0; // unknown
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(scan));
        app.view = View::Largest;
        app.by_modified = true;
        app.descending = true;
        let output = settle(&ctx, &mut app, vec2(1400.0, 900.0));
        assert_eq!(
            &app.rows[..3],
            &[5, 6, 7],
            "newest first, unknown dates last"
        );
        let drawn: Vec<_> = texts(&output).into_iter().map(|(t, _, _)| t).collect();
        for expected in ["2026-10-01", "2020-09-13", "—", "MODIFIED ↓"] {
            assert!(drawn.iter().any(|t| t == expected), "missing {expected}");
        }
        app.descending = false;
        app.dirty = true;
        settle(&ctx, &mut app, vec2(1400.0, 900.0));
        assert_eq!(&app.rows[..3], &[7, 6, 5]);
        assert_eq!(modified_day(1_790_812_800), "2026-10-01");
        assert_eq!(modified_day(0), "—");
    }
    #[test]
    fn view_switches_reuse_results_and_filters_get_their_own_keys() {
        let size = vec2(1050.0, 700.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        app.view = View::Largest;
        settle(&ctx, &mut app, size);
        let files = app.rows.clone();
        let totals = app.types_cache.clone();
        for view in [View::Folders, View::Types, View::Largest] {
            app.view = view;
            app.dirty = true;
            settle(&ctx, &mut app, size);
        }
        assert!(
            Arc::ptr_eq(&files, &app.rows),
            "Returning to Files rebuilt the list"
        );
        assert!(
            Arc::ptr_eq(&totals, &app.type_results(&ctx, true).unwrap()),
            "Types repeated the legend's aggregation"
        );
        app.query = "ext:zip".into();
        settle(&ctx, &mut app, size);
        assert_eq!(&*app.rows, &[6]);
        app.view = View::Types;
        settle(&ctx, &mut app, size);
        let filtered = app.type_results(&ctx, true).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, ".zip");
        app.query.clear();
        app.category = Some(Category::Media);
        app.view = View::Largest;
        settle(&ctx, &mut app, size);
        assert_eq!(&*app.rows, &[5]);
    }
    #[test]
    fn review_regression_unicode_search_agrees_across_map_files_and_types() {
        for metric in [Metric::Allocated, Metric::Logical] {
            for (input, expected) in [
                ("ÉTÉ", vec![1, 2]),
                ("éTÉ", vec![1, 2]),
                ("EXT:ÉTÉ", vec![2]),
                ("CAFÉ EXT:ÉTÉ >1B", vec![2]),
                ("MISSÉ", vec![]),
            ] {
                let mut scan = Scan {
                    root: "C:\\".into(),
                    stats: Default::default(),
                    nodes: vec![
                        Node::new("root".into(), None, true),
                        Node::new("été.txt".into(), Some(0), false),
                        Node::new("CAFÉ.ÉTÉ".into(), Some(0), false),
                        Node::new("other.txt".into(), Some(0), false),
                    ],
                };
                scan.nodes[0].children = vec![1, 2, 3];
                for id in 1..=3 {
                    scan.nodes[id].logical = id as u64 * 100;
                    scan.nodes[id].allocated = id as u64 * 4096;
                }
                scan.finish();
                let bytes: u64 = expected.iter().map(|&id| scan.nodes[id].size(metric)).sum();
                let ctx = egui::Context::default();
                configure_ui(&ctx);
                let size = vec2(1050.0, 700.0);
                let mut app = app(Some(scan));
                app.selected = None;
                app.metric = metric;
                app.query = input.into();
                for view in [View::Largest, View::Folders, View::Types, View::Largest] {
                    app.view = view;
                    app.dirty = true;
                    let deadline = Instant::now() + Duration::from_secs(2);
                    loop {
                        settle(&ctx, &mut app, size);
                        if app.filter_key.as_ref() == Some(&(metric, input.to_owned(), None)) {
                            break;
                        }
                        assert!(
                            Instant::now() < deadline,
                            "Filter did not finish for {input}"
                        );
                        std::thread::yield_now();
                    }
                    let mut rows = app.rows.to_vec();
                    rows.sort_unstable();
                    assert_eq!(rows, expected, "Files disagree for {input}");
                    assert_eq!(
                        app.weights.as_ref().unwrap()[0],
                        bytes,
                        "Map disagrees for {input}"
                    );
                    assert_eq!(
                        app.matching_files.as_ref().unwrap()[0],
                        expected.len() as u64
                    );
                    let types = app.type_results(&ctx, true).unwrap();
                    assert_eq!(
                        types.iter().map(|row| row.1).sum::<u64>(),
                        bytes,
                        "Types disagree for {input}"
                    );
                    assert_eq!(
                        types.iter().map(|row| row.2).sum::<u64>(),
                        expected.len() as u64
                    );
                    if view == View::Folders {
                        let mut tiles: Vec<_> = app
                            .map_cache
                            .iter()
                            .filter(|tile| tile.kind == treemap::Kind::File)
                            .map(|tile| tile.id)
                            .collect();
                        tiles.sort_unstable();
                        assert_eq!(tiles, expected, "Map tiles disagree for {input}");
                    }
                }
            }
        }
    }
    #[test]
    fn pending_file_results_leave_the_tabs_usable() {
        let size = vec2(1050.0, 700.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        app.prefetch_results = false;
        app.view = View::Largest;
        let (release, blocked) = mpsc::channel();
        let key = app.rows_key();
        app.rows_cache.request(key, true, &ctx, move || {
            blocked.recv().unwrap();
            vec![5, 6, 7].into()
        });
        let waiting = settle(&ctx, &mut app, size);
        assert!(
            texts(&waiting)
                .iter()
                .any(|(text, _, _)| text == "Preparing file list…")
        );
        click_label(&ctx, &mut app, size, "Map");
        assert!(app.view == View::Folders);
        click_label(&ctx, &mut app, size, "Types");
        assert!(app.view == View::Types);
        release.send(()).unwrap();
    }
    #[test]
    fn loading_a_new_scan_clears_old_tab_results() {
        let size = vec2(1050.0, 700.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        app.view = View::Largest;
        settle(&ctx, &mut app, size);
        assert_eq!(&*app.rows, &[5, 6, 7]);
        let mut replacement = fixture();
        replacement.nodes[5].allocated = 1;
        replacement.nodes[6].allocated = 2;
        replacement.finish();
        let (sender, receiver) = mpsc::channel();
        app.receiver = Some(receiver);
        sender.send((app.scan_request, Ok(replacement))).unwrap();
        settle(&ctx, &mut app, size);
        assert_eq!(&*app.rows, &[7, 6, 5]);
        assert_eq!(
            app.types_cache
                .iter()
                .find(|(extension, _, _)| extension == ".mp4")
                .unwrap()
                .1,
            1
        );
    }
    #[test]
    fn age_filter_drives_the_file_list_through_node_dates() {
        let mut scan = fixture();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        scan.nodes[5].modified = now - 10 * 86_400;
        scan.nodes[6].modified = now - 900 * 86_400;
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(scan));
        app.view = View::Largest;
        app.query = "older:1y".into();
        settle(&ctx, &mut app, vec2(1400.0, 900.0));
        assert_eq!(&app.rows[..], &[6], "only the file untouched for 900 days");
    }
    #[test]
    fn scan_progress_is_determinate_only_while_reading_the_index() {
        let mut app = app(None);
        assert_eq!(app.scan_fraction(), None);
        let (_sender, receiver) = mpsc::channel();
        app.receiver = Some(receiver);
        app.progress.metadata_total.store(200, Ordering::Relaxed);
        app.progress.metadata_bytes.store(50, Ordering::Relaxed);
        app.progress.phase.store(1, Ordering::Relaxed);
        assert_eq!(app.scan_fraction(), Some(0.25));
        app.progress.metadata_bytes.store(900, Ordering::Relaxed);
        assert_eq!(app.scan_fraction(), Some(1.0), "never beyond 100%");
        app.progress.phase.store(2, Ordering::Relaxed);
        assert_eq!(
            app.scan_fraction(),
            None,
            "tree building has no known total"
        );
        app.progress.phase.store(3, Ordering::Relaxed);
        assert_eq!(
            app.scan_fraction(),
            None,
            "snapshot loading has no known total"
        );
    }
    #[test]
    fn permission_errors_become_advice_with_a_relaunch_offer() {
        let denied = "MFT scan unavailable: Windows denied access to the NTFS index (Access is denied. (os error 5)). Live MFT scans require administrator access.";
        let (text, offer) = friendly_error(denied, false);
        assert!(offer);
        assert!(text.contains("administrator access") && !text.contains("os error"));
        // Already elevated: relaunching cannot help, so keep the technical text.
        assert_eq!(friendly_error(denied, true), (denied.to_owned(), false));
        let other = "NTFS index read failed: device removed";
        assert_eq!(friendly_error(other, false), (other.to_owned(), false));
    }
    #[test]
    fn error_banner_wraps_and_keeps_its_buttons_on_screen() {
        for (size, elevated) in [(vec2(1050.0, 700.0), false), (vec2(1400.0, 900.0), true)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut app = app(Some(fixture()));
            app.elevated = elevated;
            app.scan_error = Some(
                "MFT scan unavailable: Windows denied access to the NTFS index (Access is denied. (os error 5)). Live MFT scans require administrator access. Close Diskvale and start it with Run as administrator to scan.".into(),
            );
            let output = settle(&ctx, &mut app, size);
            let found = texts(&output);
            for label in ["Dismiss", "All types", "Copy path"] {
                let (_, bounds, _) = found
                    .iter()
                    .find(|(t, _, _)| t == label)
                    .unwrap_or_else(|| panic!("{label} not drawn at {size:?}"));
                assert!(
                    bounds.right() < size.x && bounds.bottom() < size.y,
                    "{label} clipped"
                );
            }
            assert_eq!(
                found.iter().any(|(t, _, _)| t == "Relaunch as admin"),
                !elevated,
                "relaunch offered only when it can help"
            );
            // The banner must not squeeze the map away.
            let map = app
                .map_cache
                .iter()
                .map(tile_rect)
                .fold(Rect::NOTHING, |a, b| a.union(b));
            assert!(
                map.height() >= 200.0,
                "map squeezed to {} px at {size:?}",
                map.height()
            );
        }
    }
    #[test]
    fn file_edges_are_visible_but_subtle_for_every_category() {
        for category in Category::ALL {
            for area in [900.0, 10_000.0] {
                let fill = muted_tile(category_color(category), area);
                let edge = tile_edge(fill);
                let ratio = contrast(fill, edge);
                assert!(
                    (1.12..1.8).contains(&ratio),
                    "{}: edge contrast {ratio:.2} is invisible or heavy",
                    category.label()
                );
            }
        }
    }
    fn fill_mesh(app: &Canopy) -> egui::Mesh {
        app.map_shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Mesh(mesh) => Some((**mesh).clone()),
                _ => None,
            })
            .expect("the map caches one fill mesh")
    }
    #[test]
    fn large_files_get_a_cushion_and_subpixel_files_stay_unbevelled() {
        let size = vec2(1400.0, 900.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        settle(&ctx, &mut app, size);
        let block = tile_rect(app.map_cache.iter().find(|t| t.id == 5).unwrap());
        assert!(block.width() >= FILE_EDGE_MIN && block.height() >= FILE_EDGE_MIN);
        let fill = muted_tile(file_color("capture.mp4"), block.area());
        let colors: Vec<_> = fill_mesh(&app).vertices.iter().map(|v| v.color).collect();
        assert!(
            colors.contains(&fill) && colors.contains(&tile_edge(fill)),
            "large file lacks its edge"
        );
        // Shrink the same file below the threshold: it must not be shaded.
        let mut scan = fixture();
        scan.nodes[5].allocated = 1;
        scan.nodes[5].logical = 1;
        scan.finish();
        let mut app = super::tests::app(Some(scan));
        settle(&ctx, &mut app, size);
        if let Some(tile) = app.map_cache.iter().find(|t| t.id == 5) {
            let block = tile_rect(tile);
            assert!(block.width() < FILE_EDGE_MIN || block.height() < FILE_EDGE_MIN);
            let tiny = muted_tile(file_color("capture.mp4"), block.area());
            assert!(
                !fill_mesh(&app)
                    .vertices
                    .iter()
                    .any(|v| v.color == tile_edge(tiny)),
                "a tiny file was shaded"
            );
        }
    }
    #[test]
    fn small_file_boundaries_stay_inside_real_tiles_at_each_pixel_density() {
        let fill = file_color("capture.mp4");
        for density in [1.0, 1.25, 2.0] {
            for side in [4.0, 8.0, 20.0] {
                let block = Rect::from_min_size(egui::pos2(81.3, 37.2), vec2(side, side));
                let mut mesh = egui::Mesh::default();
                add_file_surface(&mut mesh, block, fill, 17, density);
                assert!(mesh.is_valid());
                assert!(mesh.vertices.iter().all(|v| block.contains(v.pos)));
                assert!(
                    mesh.vertices.iter().any(|v| v.color == tile_edge(fill)),
                    "A {side}-point file lost its boundary at {density} pixels per point"
                );
            }
        }
        let block = Rect::from_min_size(egui::Pos2::ZERO, vec2(1.0, 1.0));
        let mut a = egui::Mesh::default();
        let mut b = egui::Mesh::default();
        add_file_surface(&mut a, block, fill, 1, 1.0);
        add_file_surface(&mut b, block, fill, 2, 1.0);
        assert_eq!(
            a.vertices.len(),
            4,
            "Subpixel files should not become a heavy bevel grid"
        );
        assert_ne!(
            a.vertices[0].color, b.vertices[0].color,
            "A dense same-type patch has no tone cue"
        );
    }
    #[test]
    fn grouped_texture_is_clipped_and_bounded_and_counts_fit_a_59_point_box() {
        let block = Rect::from_min_size(egui::pos2(13.0, 27.0), vec2(59.0, 60.0));
        let mut mesh = egui::Mesh::default();
        let mut budget = 3;
        add_group_pattern(&mut mesh, block, 1.0, &mut budget);
        assert_eq!(budget, 0);
        assert!(mesh.is_valid() && !mesh.indices.is_empty());
        assert!(mesh.vertices.iter().all(|v| block.contains(v.pos)));
        let before = mesh.vertices.len();
        add_group_pattern(&mut mesh, block, 1.0, &mut budget);
        assert_eq!(
            mesh.vertices.len(),
            before,
            "The global pattern budget was exceeded"
        );
        let ctx = egui::Context::default();
        let output = ctx.run(frame_input(vec2(300.0, 200.0), vec![]), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut mesh = egui::Mesh::default();
                let mut labels = vec![];
                add_group_badge(ui.painter(), &mut mesh, &mut labels, block, 2480, false);
                assert!(mesh.vertices.iter().all(|v| block.contains(v.pos)));
                ui.painter().extend(labels);
            });
        });
        let labels = texts(&output);
        assert!(labels.iter().any(|(text, _, _)| text.starts_with("2.5k")));
        assert!(labels.iter().all(|(_, rect, _)| block.contains_rect(*rect)));
    }
    #[test]
    fn collapsed_folders_use_group_texture_and_a_file_count() {
        let size = vec2(1050.0, 700.0);
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(fixture()));
        app.depth = 1;
        let output = settle(&ctx, &mut app, size);
        assert!(texts(&output).iter().any(|(text, _, _)| text == "2 files"));
        assert!(
            fill_mesh(&app)
                .vertices
                .iter()
                .any(|v| v.color == Color32::from_rgb(63, 78, 92))
        );
        app.category = Some(Category::Media);
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.matching_files.is_none() {
            settle(&ctx, &mut app, size);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let filtered = settle(&ctx, &mut app, size);
        assert!(
            texts(&filtered)
                .iter()
                .any(|(text, _, _)| text == "1 matching file")
        );
        assert_eq!(
            app.scan.as_ref().unwrap().nodes[1].files,
            2,
            "Filtering should not change the scan's original file count"
        );
    }
    #[test]
    fn group_counts_take_priority_over_icons_and_scale_without_clipping() {
        let ctx = egui::Context::default();
        for (size, files, filtered, expected) in [
            (vec2(220.0, 120.0), 12874, false, Some("12,874 files")),
            (
                vec2(220.0, 120.0),
                12874,
                true,
                Some("12,874 matching files"),
            ),
            (vec2(36.0, 96.0), 31874, false, Some("32k")),
            (vec2(36.0, 96.0), 87, false, Some("87")),
            (vec2(24.0, 30.0), 12874, false, None),
        ] {
            let block = Rect::from_min_size(egui::pos2(13.0, 27.0), size);
            let output = ctx.run(frame_input(vec2(400.0, 240.0), vec![]), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut mesh = egui::Mesh::default();
                    let mut labels = vec![];
                    add_group_badge(ui.painter(), &mut mesh, &mut labels, block, files, filtered);
                    assert!(mesh.is_valid() && mesh.vertices.iter().all(|v| block.contains(v.pos)));
                    ui.painter().extend(labels);
                });
            });
            let labels = texts(&output);
            if let Some(expected) = expected {
                assert!(
                    labels.iter().any(|(text, _, _)| text == expected),
                    "Missing count {expected:?} in {size:?}: {labels:?}"
                );
                assert!(labels.iter().all(|(_, rect, _)| block.contains_rect(*rect)));
                if size.x > 200.0 {
                    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.sections.iter().all(|section| section.format.font_id.size >= 14.0))), "The large group kept its old tiny type");
                }
            } else {
                assert!(
                    labels.is_empty(),
                    "Tiny groups should keep a quiet icon-only marker"
                );
            }
        }
    }
    #[test]
    fn cushion_mesh_fills_the_original_rectangle_without_spilling() {
        let block = Rect::from_min_size(egui::pos2(81.5, 37.0), vec2(137.0, 89.5));
        let mut mesh = egui::Mesh::default();
        add_cushion(&mut mesh, block, file_color("example.mp4"));
        assert!(mesh.is_valid());
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| block.contains(vertex.pos))
        );
        let area: f32 = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|triangle| {
                let a = mesh.vertices[triangle[0] as usize].pos;
                let b = mesh.vertices[triangle[1] as usize].pos;
                let c = mesh.vertices[triangle[2] as usize].pos;
                ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5
            })
            .sum();
        assert!(
            (area - block.area()).abs() < 0.01,
            "Shading changed the byte-proportional area"
        );
    }
    #[test]
    fn tooltip_uses_file_size_without_extra_statistics() {
        let size = vec2(1400.0, 900.0);
        let mut scan = fixture();
        // pagefile.sys: the two measures differ by a few bytes, so they print the same.
        scan.nodes[7].logical = scan.nodes[7].allocated - 5;
        scan.finish();
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(scan));
        settle(&ctx, &mut app, size);
        let tooltip = |app: &mut Canopy, id: usize| {
            let point = center_of(app, id);
            let mut output = None;
            for _ in 0..3 {
                output = Some(ctx.run(
                    frame_input(size, vec![egui::Event::PointerMoved(point)]),
                    |ctx| app.render(ctx),
                ));
            }
            texts(&output.unwrap())
                .into_iter()
                .map(|(text, _, _)| text)
                .collect::<Vec<_>>()
        };
        let same = tooltip(&mut app, 7);
        assert!(same.iter().any(|t| t == "File size: 10.74 GB"), "{same:?}");
        assert!(
            !same
                .iter()
                .any(|t| t.starts_with("Size:") || t.starts_with("Size on disk:")),
            "Added a second size value: {same:?}"
        );
        // capture.mp4 is 40 GiB on disk, but hover shows its requested 41 GiB file size.
        let different = tooltip(&mut app, 5);
        assert!(
            different.iter().any(|t| t == "File size: 44.02 GB"),
            "{different:?}"
        );
        assert!(
            different
                .iter()
                .any(|t| t == "Location: C:\\Users\\Guest\\Desktop\\Projects")
        );
        assert!(!different.iter().any(|t| t.contains("of indexed drive")
            || t.starts_with("Type:")
            || t.contains("right-click for more")));
        app.metric = Metric::Logical;
        app.invalidate_metric();
        settle(&ctx, &mut app, size);
        let logical = tooltip(&mut app, 5);
        assert!(logical.iter().any(|t| t == "File size: 44.02 GB"));
    }
    #[test]
    fn documents_and_other_are_no_longer_two_greys() {
        let [r, g, b] = Category::Documents.rgb();
        assert!(
            g > r.saturating_add(60) && g > b,
            "Documents should be a clear green"
        );
        let grey = |rgb: [u8; 3]| rgb.iter().max().unwrap() - rgb.iter().min().unwrap() < 40;
        let greys = Category::ALL.iter().filter(|c| grey(c.rgb())).count();
        assert_eq!(greys, 1, "only Other may be grey");
        // Other must also stand apart from the grouped-files fill.
        assert!(contrast(category_color(Category::Other), GROUPED) > 1.6);
    }
    #[test]
    fn palette_has_readable_labels_on_muted_and_shaded_tiles() {
        let mut colors = HashSet::new();
        for category in Category::ALL {
            assert!(colors.insert(category.rgb()));
            for area in [10.0, 100.0, 10000.0] {
                let fill = muted_tile(category_color(category), area);
                assert_eq!(fill.a(), 255);
                assert!(
                    contrast(fill, tile_ink(fill)) >= 4.5,
                    "{}",
                    category.label()
                );
                if area >= FILE_EDGE_MIN * FILE_EDGE_MIN {
                    let mut mesh = egui::Mesh::default();
                    add_cushion(
                        &mut mesh,
                        Rect::from_min_size(egui::Pos2::ZERO, vec2(100.0, 100.0)),
                        fill,
                    );
                    for vertex in mesh.vertices {
                        assert_eq!(vertex.color.a(), 255);
                        assert!(
                            contrast(vertex.color, tile_ink(fill)) >= 4.5,
                            "{} shaded label lost contrast",
                            category.label()
                        );
                    }
                }
            }
        }
        assert_eq!(modified(0), "Unavailable");
        assert_eq!(modified(1_790_812_800), "2026-10-01 UTC");
    }
    #[test]
    #[ignore = "Explicit full saved-scan UI timing; set CANOPY_TIMING_SNAPSHOT and CANOPY_TIMING_OUTPUT"]
    fn measure_saved_scan_view_switches() {
        let path = std::env::var("CANOPY_TIMING_SNAPSHOT").expect("saved snapshot path");
        let scan: Scan =
            serde_json::from_reader(std::io::BufReader::new(std::fs::File::open(path).unwrap()))
                .unwrap();
        let files = scan.stats.files;
        let nodes = scan.nodes.len();
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app(Some(scan));
        app.selected = None;
        let size = vec2(1400.0, 900.0);
        let mut timings = vec![];
        for (name, view) in [
            ("Map initial", View::Folders),
            ("Files first", View::Largest),
            ("Types first", View::Types),
            ("Map return", View::Folders),
            ("Files return", View::Largest),
            ("Types return", View::Types),
            ("Map again", View::Folders),
        ] {
            app.view = view;
            app.dirty = true;
            let started = Instant::now();
            let _ = ctx.run(frame_input(size, vec![]), |ctx| app.render(ctx));
            let first_ms = started.elapsed().as_secs_f64() * 1000.0;
            let mut max_ms = first_ms;
            let deadline = Instant::now() + Duration::from_secs(30);
            while app.extensions_cache.pending()
                || (view == View::Largest && app.rows_cache.pending())
            {
                assert!(Instant::now() < deadline, "Background view work timed out");
                std::thread::sleep(Duration::from_millis(10));
                let frame = Instant::now();
                let _ = ctx.run(frame_input(size, vec![]), |ctx| app.render(ctx));
                max_ms = max_ms.max(frame.elapsed().as_secs_f64() * 1000.0);
            }
            let ready_ms = started.elapsed().as_secs_f64() * 1000.0;
            for _ in 0..3 {
                let started = Instant::now();
                let _ = ctx.run(frame_input(size, vec![]), |ctx| app.render(ctx));
                max_ms = max_ms.max(started.elapsed().as_secs_f64() * 1000.0);
            }
            eprintln!("{name}: {first_ms:.2} ms first / {max_ms:.2} ms max");
            timings.push(
                serde_json::json!({"view":name,"first_frame_ms":first_ms,"max_frame_ms":max_ms,"data_ready_ms":ready_ms}),
            );
        }
        let output = std::env::var("CANOPY_TIMING_OUTPUT").expect("timing output path");
        std::fs::write(output, serde_json::to_vec_pretty(&serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"files":files,"nodes":nodes,"renderer":"production egui CPU frames, no GPU presentation","timings":timings})).unwrap()).unwrap();
    }
}
