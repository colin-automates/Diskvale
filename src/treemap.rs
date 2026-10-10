use crate::model::{Metric, Scan};
use std::collections::BinaryHeap;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    Aggregate,
}

#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub id: usize,
    pub rect: Rect,
    pub depth: u16,
    pub kind: Kind,
    pub size: u64,
    pub files: u64,
    /// A reserved folder heading; file content never paints underneath it.
    pub header: bool,
}

/// Height of the label strip reserved at the top of a headed folder.
pub const HEADER_HEIGHT: f32 = 18.0;

/// Readable folders can have a heading at any depth. The layout also limits
/// the cumulative heading height along a branch to leave room for its files.
pub fn has_header(depth: u16, rect: Rect) -> bool {
    depth > 0 && rect.w >= 104.0 && rect.h >= 64.0 && rect.w * rect.h >= 9000.0
}

/// Every folder contains its descendants. Tiny regions retain their folder's
/// total area; the largest visible regions receive detail first.
pub fn nested(scan: &Scan, root: usize, metric: Metric, rect: Rect) -> Vec<Tile> {
    nested_with_budget(scan, root, metric, rect, 30_000)
}

#[derive(Clone, Copy)]
pub struct Options<'a> {
    pub weights: Option<&'a [u64]>,
    /// Counts from the same predicate as weights, when the map is filtered.
    pub counts: Option<&'a [u64]>,
    pub depth: u16,
    pub minimum: u64,
}
impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            weights: None,
            counts: None,
            depth: 96,
            minimum: 0,
        }
    }
}
pub fn with_options(
    scan: &Scan,
    root: usize,
    metric: Metric,
    rect: Rect,
    options: Options<'_>,
) -> Vec<Tile> {
    build(scan, root, metric, rect, 30_000, options)
}

fn nested_with_budget(
    scan: &Scan,
    root: usize,
    metric: Metric,
    rect: Rect,
    budget: usize,
) -> Vec<Tile> {
    build(scan, root, metric, rect, budget, Options::default())
}

fn build(
    scan: &Scan,
    root: usize,
    metric: Metric,
    rect: Rect,
    budget: usize,
    options: Options<'_>,
) -> Vec<Tile> {
    let size = |id: usize| {
        options
            .weights
            .map_or_else(|| scan.nodes[id].size(metric), |w| w[id])
    };
    let files = |id: usize| {
        options
            .counts
            .map_or(scan.nodes[id].files, |counts| counts[id])
    };
    if size(root) == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return vec![];
    }
    let mut tiles = vec![Tile {
        id: root,
        rect,
        depth: 0,
        kind: Kind::Folder,
        size: size(root),
        files: files(root),
        header: false,
    }];
    // Enough ancestry for meaningful names (e.g. the games below steamapps),
    // while preserving most of the view for file content even in a deep chain.
    let max_headers = (rect.h * 0.35 / HEADER_HEIGHT).floor().min(7.0) as u16;
    let mut queue = BinaryHeap::from([((rect.w * rect.h).to_bits(), 0usize, 0u16)]);
    while let Some((_, index, used_headers)) = queue.pop() {
        let parent = tiles[index];
        let slots = budget.saturating_sub(tiles.len());
        if slots < 2
            || parent.rect.w < 3.0
            || parent.rect.h < 3.0
            || parent.depth >= options.depth
            || parent.size < options.minimum
        {
            continue;
        }
        let header = if parent.header { HEADER_HEIGHT } else { 0.0 };
        let inner = Rect {
            x: parent.rect.x,
            y: parent.rect.y + header,
            w: parent.rect.w,
            h: parent.rect.h - header,
        };
        let mut items: Vec<_> = scan.nodes[parent.id]
            .children
            .iter()
            .map(|&id| (id, size(id)))
            .filter(|(_, size)| *size > 0)
            .collect();
        items.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut grouped_files = 0;
        // A huge directory must not spend the entire global budget before
        // other large branches are expanded. Allocate detail by visible area
        // and bound per-folder fanout; the remaining bytes stay in one tile.
        let area = inner.w * inner.h;
        let quota = (area / (rect.w * rect.h) * budget as f32).ceil() as usize;
        let pixel_limit = (area / 4.0).ceil() as usize;
        let limit = slots.min(512).min(quota.max(2)).min(pixel_limit.max(2));
        let visible = items
            .iter()
            .take_while(|(_, bytes)| *bytes >= options.minimum)
            .count()
            .min(limit - 1);
        if items.len() > limit || visible < items.len() {
            let grouped = items.split_off(visible);
            let size = grouped.iter().map(|(_, size)| *size).sum();
            grouped_files = grouped.iter().map(|(id, _)| files(*id)).sum();
            items.push((usize::MAX, size));
        }
        for (id, rect) in layout(&items, inner) {
            let aggregate = id == usize::MAX;
            let id = if aggregate { parent.id } else { id };
            let kind = if aggregate {
                Kind::Aggregate
            } else if scan.nodes[id].directory {
                Kind::Folder
            } else {
                Kind::File
            };
            let next = tiles.len();
            let headed = kind == Kind::Folder
                && used_headers < max_headers
                && has_header(parent.depth + 1, rect);
            tiles.push(Tile {
                id,
                rect,
                depth: parent.depth + 1,
                kind,
                size: if aggregate {
                    items.last().unwrap().1
                } else {
                    size(id)
                },
                files: if aggregate { grouped_files } else { files(id) },
                header: headed,
            });
            if kind == Kind::Folder && rect.w * rect.h >= 8.0 {
                queue.push((
                    (rect.w * rect.h).to_bits(),
                    next,
                    used_headers + u16::from(headed),
                ));
            }
        }
    }
    tiles
}

/// Squarified layout (Bruls, Huizing and van Wijk). Items fill strips along the
/// shorter side of the free space, and each strip stops growing once another
/// item would make its worst tile less square. Tiles stay close to square, so
/// names fit and areas compare easily. The result is gapless and exactly
/// area-proportional. Runs in O(n log n).
pub fn layout(items: &[(usize, u64)], rect: Rect) -> Vec<(usize, Rect)> {
    let mut items: Vec<_> = items.iter().copied().filter(|x| x.1 > 0).collect();
    let mut output = Vec::with_capacity(items.len());
    if items.is_empty() || rect.w <= 0.0 || rect.h <= 0.0 {
        return output;
    }
    // Stable, so equal sizes keep the caller's order.
    items.sort_by_key(|item| std::cmp::Reverse(item.1));
    let total = items.iter().map(|x| x.1 as f64).sum::<f64>();
    let scale = rect.w as f64 * rect.h as f64 / total;
    let (mut x, mut y) = (rect.x as f64, rect.y as f64);
    let (mut w, mut h) = (rect.w as f64, rect.h as f64);
    let mut start = 0;
    while start < items.len() && w > 0.0 && h > 0.0 {
        let side = w.min(h);
        let (mut sum, mut smallest, mut largest) = (0.0, f64::MAX, 0.0_f64);
        let mut worst = f64::MAX;
        let mut end = start;
        while end < items.len() {
            let area = items[end].1 as f64 * scale;
            let (next_sum, lo, hi) = (sum + area, smallest.min(area), largest.max(area));
            let ratio = (side * side * hi / (next_sum * next_sum))
                .max(next_sum * next_sum / (side * side * lo));
            if end > start && ratio > worst {
                break;
            }
            (sum, smallest, largest, worst) = (next_sum, lo, hi, ratio);
            end += 1;
        }
        let row = &items[start..end];
        // The final strip takes whatever is left, so float drift cannot open a gap.
        let last = end == items.len();
        if w >= h {
            let strip = if last { w } else { (sum / h).min(w) };
            let mut top = y;
            for (i, (id, size)) in row.iter().enumerate() {
                let tile_h = if i + 1 == row.len() {
                    y + h - top
                } else {
                    *size as f64 * scale / strip
                };
                output.push((
                    *id,
                    Rect {
                        x: x as f32,
                        y: top as f32,
                        w: strip as f32,
                        h: tile_h as f32,
                    },
                ));
                top += tile_h;
            }
            x += strip;
            w -= strip;
        } else {
            let strip = if last { h } else { (sum / w).min(h) };
            let mut left = x;
            for (i, (id, size)) in row.iter().enumerate() {
                let tile_w = if i + 1 == row.len() {
                    x + w - left
                } else {
                    *size as f64 * scale / strip
                };
                output.push((
                    *id,
                    Rect {
                        x: left as f32,
                        y: y as f32,
                        w: tile_w as f32,
                        h: strip as f32,
                    },
                ));
                left += tile_w;
            }
            y += strip;
            h -= strip;
        }
        start = end;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Node, ScanStats};
    fn tree() -> Scan {
        let mut scan = Scan {
            root: "fixture".into(),
            stats: ScanStats::default(),
            nodes: vec![
                Node::new("fixture".into(), None, true),
                Node::new("project".into(), Some(0), true),
                Node::new("nested".into(), Some(1), true),
                Node::new("large.zip".into(), Some(2), false),
                Node::new("small.png".into(), Some(1), false),
            ],
        };
        scan.nodes[0].children = vec![1];
        scan.nodes[1].children = vec![2, 4];
        scan.nodes[2].children = vec![3];
        scan.nodes[3].allocated = 900;
        scan.nodes[4].allocated = 100;
        scan.finish();
        scan
    }
    #[test]
    fn nested_children_touch_folder_edges_and_fill_the_content_area() {
        let scan = tree();
        let tiles = nested(
            &scan,
            0,
            Metric::Allocated,
            Rect {
                x: 12.0,
                y: 24.0,
                w: 800.0,
                h: 400.0,
            },
        );
        for id in [0, 1, 2] {
            let parent = tiles
                .iter()
                .find(|tile| tile.id == id && tile.kind == Kind::Folder)
                .unwrap();
            let children: Vec<_> = scan.nodes[id]
                .children
                .iter()
                .map(|child| tiles.iter().find(|tile| tile.id == *child).unwrap())
                .collect();
            let left = children
                .iter()
                .map(|tile| tile.rect.x)
                .fold(f32::INFINITY, f32::min);
            let right = children
                .iter()
                .map(|tile| tile.rect.x + tile.rect.w)
                .fold(f32::NEG_INFINITY, f32::max);
            let top = children
                .iter()
                .map(|tile| tile.rect.y)
                .fold(f32::INFINITY, f32::min);
            let bottom = children
                .iter()
                .map(|tile| tile.rect.y + tile.rect.h)
                .fold(f32::NEG_INFINITY, f32::max);
            assert!((left - parent.rect.x).abs() < 0.01);
            assert!((right - parent.rect.x - parent.rect.w).abs() < 0.01);
            let header = if parent.header { HEADER_HEIGHT } else { 0.0 };
            assert!((top - parent.rect.y - header).abs() < 0.01);
            assert!((bottom - parent.rect.y - parent.rect.h).abs() < 0.01);
            let occupied: f32 = children.iter().map(|tile| tile.rect.w * tile.rect.h).sum();
            assert!((occupied - parent.rect.w * (parent.rect.h - header)).abs() < 1.0);
        }
    }
    #[test]
    fn deep_folder_headers_leave_file_content_and_preserve_zoomed_detail() {
        let mut scan = tree();
        scan.nodes.truncate(1);
        scan.nodes[0].children.clear();
        for id in 1..=18 {
            scan.nodes
                .push(Node::new(format!("folder-{id}"), Some(id - 1), true));
            scan.nodes[id - 1].children.push(id);
        }
        let mut file = Node::new("large.zip".into(), Some(18), false);
        file.allocated = 1000;
        scan.nodes.push(file);
        scan.nodes[18].children.push(19);
        scan.finish();
        for height in [101.0, 320.0, 540.0] {
            let rect = Rect {
                x: 10.0,
                y: 20.0,
                w: 800.0,
                h: height,
            };
            let tiles = nested(&scan, 0, Metric::Allocated, rect);
            if height >= 320.0 {
                assert!(tiles.iter().any(|tile| tile.depth > 2 && tile.header));
            }
            let leaf = tiles.iter().find(|tile| tile.kind == Kind::File).unwrap();
            assert!(
                leaf.rect.h >= height * 0.65,
                "headers consumed file content"
            );
            for parent in tiles.iter().filter(|tile| tile.header) {
                let child = tiles.iter().find(|tile| tile.id == parent.id + 1).unwrap();
                assert!(child.rect.y >= parent.rect.y + HEADER_HEIGHT);
            }
            let zoomed = nested(&scan, 10, Metric::Allocated, rect);
            assert!(zoomed.iter().any(|tile| tile.id == 11 && tile.header));
            assert_eq!(zoomed.last().unwrap().size, 1000);
        }
    }
    #[test]
    fn whole_tree_contains_grandchild_files_inside_their_ancestors() {
        let scan = tree();
        let tiles = nested(
            &scan,
            0,
            Metric::Allocated,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 400.0,
            },
        );
        let leaf = tiles.iter().find(|tile| tile.id == 3).unwrap();
        assert_eq!(leaf.kind, Kind::File);
        for id in [0, 1, 2] {
            let ancestor = tiles.iter().find(|tile| tile.id == id).unwrap();
            assert!(leaf.rect.x >= ancestor.rect.x && leaf.rect.y >= ancestor.rect.y);
            assert!(leaf.rect.x + leaf.rect.w <= ancestor.rect.x + ancestor.rect.w + 0.01);
            assert!(leaf.rect.y + leaf.rect.h <= ancestor.rect.y + ancestor.rect.h + 0.01);
        }
        assert!(
            tiles
                .iter()
                .any(|tile| tile.id == 4 && tile.kind == Kind::File)
        );
    }
    #[test]
    fn render_budget_preserves_small_file_bytes_in_an_aggregate() {
        let mut scan = tree();
        scan.nodes.truncate(1);
        scan.nodes[0].children.clear();
        for i in 1..=100 {
            let mut node = Node::new(format!("{i}.bin"), Some(0), false);
            node.allocated = i as u64;
            scan.nodes.push(node);
            scan.nodes[0].children.push(i);
        }
        scan.finish();
        let tiles = nested_with_budget(
            &scan,
            0,
            Metric::Allocated,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 400.0,
            },
            24,
        );
        assert_eq!(tiles.len(), 24);
        assert_eq!(
            tiles.iter().skip(1).map(|tile| tile.size).sum::<u64>(),
            5050
        );
        assert_eq!(
            tiles.iter().skip(1).map(|tile| tile.files).sum::<u64>(),
            100
        );
        assert!(tiles.iter().any(|tile| tile.kind == Kind::Aggregate));
    }
    #[test]
    fn wide_folder_leaves_detail_for_other_large_branches() {
        let mut scan = tree();
        scan.nodes.truncate(3);
        scan.nodes[0].children = vec![1, 2];
        scan.nodes[2].parent = Some(0);
        scan.nodes[1].children.clear();
        scan.nodes[2].children.clear();
        for i in 0..1000 {
            let id = scan.nodes.len();
            let mut node = Node::new(format!("{i}.bin"), Some(1), false);
            node.allocated = 1;
            scan.nodes.push(node);
            scan.nodes[1].children.push(id);
        }
        let large = scan.nodes.len();
        let mut node = Node::new("large.zip".into(), Some(2), false);
        node.allocated = 500;
        scan.nodes.push(node);
        scan.nodes[2].children.push(large);
        scan.finish();
        let tiles = nested_with_budget(
            &scan,
            0,
            Metric::Allocated,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 400.0,
            },
            64,
        );
        assert!(tiles.len() <= 64);
        assert!(
            tiles
                .iter()
                .any(|tile| tile.id == large && tile.kind == Kind::File)
        );
        assert_eq!(
            tiles
                .iter()
                .filter(|tile| tile.kind != Kind::Folder)
                .map(|tile| tile.size)
                .sum::<u64>(),
            1500
        );
    }
    #[test]
    fn squarified_layout_tiles_exactly_and_keeps_large_tiles_readable() {
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            w: 900.0,
            h: 500.0,
        };
        // A heavy tail like a real drive: a few giants and many small files.
        let items: Vec<_> = (0..200)
            .map(|i| (i, 2_000_000_000 / (i as u64 + 1).pow(2) + 1))
            .collect();
        let output = layout(&items, rect);
        assert_eq!(output.len(), items.len());
        let total: u64 = items.iter().map(|x| x.1).sum();
        let mut covered = 0.0_f64;
        for (id, r) in &output {
            assert!(r.x >= rect.x - 0.01 && r.y >= rect.y - 0.01);
            assert!(r.x + r.w <= rect.x + rect.w + 0.01);
            assert!(r.y + r.h <= rect.y + rect.h + 0.01);
            let expected = items[*id].1 as f64 / total as f64 * (rect.w * rect.h) as f64;
            assert!(
                (r.w as f64 * r.h as f64 - expected).abs() < 0.5,
                "area of {id}"
            );
            covered += r.w as f64 * r.h as f64;
        }
        assert!(
            (covered - (rect.w * rect.h) as f64).abs() < 1.0,
            "gap or overlap"
        );
        for (i, (_, a)) in output.iter().enumerate() {
            for (_, b) in &output[i + 1..] {
                let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
                let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
                assert!(w <= 0.01 || h <= 0.01, "tiles overlap");
            }
        }
        // Tiles big enough to carry a label must stay close to square. On this
        // data the previous binary split produced a 3.3:1 tile; squarifying
        // keeps the worst at about 1.8:1.
        for (id, r) in output.iter().filter(|(_, r)| r.w * r.h >= 1500.0) {
            let aspect = (r.w / r.h).max(r.h / r.w);
            assert!(aspect < 2.5, "tile {id} is {:.0}x{:.0}", r.w, r.h);
        }
    }
    #[test]
    fn area_is_proportional_and_zero_files_are_omitted() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        };
        let output = layout(&[(0, 60), (1, 30), (2, 10), (3, 0)], rect);
        assert_eq!(output.len(), 3);
        for (id, r) in output {
            assert!((r.w * r.h - 800.0 * [60.0, 30.0, 10.0][id]).abs() < 0.1);
            assert!(r.x + r.w <= 400.1);
            assert!(r.y + r.h <= 200.1);
        }
    }
    #[test]
    fn filtered_group_counts_and_bytes_follow_the_same_predicate() {
        let mut scan = tree();
        let mut second = Node::new("second.png".into(), Some(1), false);
        second.allocated = 100;
        scan.nodes.push(second);
        scan.nodes[1].children.push(5);
        scan.finish();
        let totals = crate::query::map_totals(
            &scan,
            Metric::Allocated,
            &crate::query::Query::parse("ext:png"),
            None,
        );
        let tiles = build(
            &scan,
            0,
            Metric::Allocated,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 100.0,
            },
            20,
            Options {
                weights: Some(&totals.bytes),
                counts: Some(&totals.files),
                minimum: 150,
                ..Default::default()
            },
        );
        let group = tiles
            .iter()
            .find(|tile| tile.kind == Kind::Aggregate)
            .unwrap();
        assert_eq!(group.size, 200);
        assert_eq!(
            group.files, 2,
            "The group should not count the excluded large file"
        );
        assert_eq!(tiles[0].files, 2);
    }
}
