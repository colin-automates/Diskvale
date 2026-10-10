use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Allocated,
    Logical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Units {
    Binary,
    #[default]
    Decimal,
}
impl Units {
    pub fn label(self) -> &'static str {
        match self {
            Self::Binary => "GiB",
            Self::Decimal => "GB",
        }
    }
    pub fn format(self, value: u64) -> String {
        let (base, labels) = match self {
            Self::Binary => (1024.0, ["B", "KiB", "MiB", "GiB", "TiB", "PiB"]),
            Self::Decimal => (1000.0, ["B", "KB", "MB", "GB", "TB", "PB"]),
        };
        let mut size = value as f64;
        let mut i = 0;
        while size >= base && i < labels.len() - 1 {
            size /= base;
            i += 1;
        }
        if i == 0 {
            format!("{value} B")
        } else {
            format!("{size:.2} {}", labels[i])
        }
    }
}

/// Physical space sampled at the end of the same live volume scan.
/// The start free-space sample exposes changes during that interval.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VolumeSpace {
    pub capacity: u64,
    pub free: u64,
    pub free_at_start: u64,
    pub captured_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub directory: bool,
    pub logical: u64,
    pub allocated: u64,
    pub files: u64,
    pub modified: u64,
    pub shared: bool,
    pub allocation_unknown: bool,
}

impl Node {
    pub fn new(name: String, parent: Option<usize>, directory: bool) -> Self {
        Self {
            name,
            parent,
            directory,
            children: vec![],
            logical: 0,
            allocated: 0,
            files: u64::from(!directory),
            modified: 0,
            shared: false,
            allocation_unknown: false,
        }
    }
    pub fn size(&self, metric: Metric) -> u64 {
        match metric {
            Metric::Allocated => self.allocated,
            Metric::Logical => self.logical,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanStats {
    #[serde(default)]
    pub volume_space: Option<VolumeSpace>,
    pub files: u64,
    pub folders: u64,
    pub skipped_links: u64,
    pub errors: u64,
    pub unknown_allocations: u64,
    pub shared_links: u64,
    pub elapsed_seconds: f64,
    pub cancelled: bool,
    pub engine: String,
    pub notices: Vec<String>,
    #[serde(default)]
    pub directory_batches: u64,
    #[serde(default)]
    pub file_metadata_queries: u64,
    #[serde(default)]
    pub metadata_bytes: u64,
    #[serde(default)]
    pub mft_records: u64,
    #[serde(default)]
    pub mft_read_seconds: f64,
    #[serde(default)]
    pub tree_build_seconds: f64,
    #[serde(default)]
    pub unlinked_records: u64,
    #[serde(default)]
    pub unlinked_logical_bytes: u64,
    #[serde(default)]
    pub unlinked_allocated_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scan {
    pub root: PathBuf,
    pub nodes: Vec<Node>,
    pub stats: ScanStats,
}

impl Scan {
    pub fn outside_indexed(&self) -> Option<u64> {
        let space = self.stats.volume_space.as_ref()?;
        if self.stats.cancelled || self.nodes[0].allocation_unknown {
            return None;
        }
        space
            .capacity
            .checked_sub(space.free)?
            .checked_sub(self.nodes[0].allocated)
    }
    pub fn path(&self, mut id: usize) -> PathBuf {
        let mut parts = vec![];
        while let Some(parent) = self.nodes[id].parent {
            parts.push(self.nodes[id].name.as_str());
            id = parent;
        }
        let mut path = self.root.clone();
        for part in parts.into_iter().rev() {
            path.push(part);
        }
        path
    }
    pub fn relative_path(&self, id: usize) -> String {
        self.path(id)
            .strip_prefix(&self.root)
            .unwrap_or(&self.root)
            .display()
            .to_string()
    }
    pub fn finish(&mut self) {
        // Postorder supports both directory traversal and arbitrarily ordered MFT records.
        let mut stack = vec![(0, false)];
        while let Some((id, visited)) = stack.pop() {
            if !visited {
                stack.push((id, true));
                for &child in &self.nodes[id].children {
                    stack.push((child, false));
                }
            } else if self.nodes[id].directory {
                let (mut logical, mut allocated, mut files, mut unknown) =
                    (0u64, 0u64, 0u64, false);
                for &child in &self.nodes[id].children {
                    logical = logical.saturating_add(self.nodes[child].logical);
                    allocated = allocated.saturating_add(self.nodes[child].allocated);
                    files += self.nodes[child].files;
                    unknown |= self.nodes[child].allocation_unknown;
                }
                let node = &mut self.nodes[id];
                node.logical = logical;
                node.allocated = allocated;
                node.files = files;
                node.allocation_unknown = unknown;
            }
        }
    }
    pub fn extensions(&self, metric: Metric) -> Vec<(String, u64, u64)> {
        self.extensions_under(0, metric)
    }
    pub fn extensions_under(&self, root: usize, metric: Metric) -> Vec<(String, u64, u64)> {
        self.extensions_matching(root, metric, |_| true)
    }
    pub fn extensions_matching(
        &self,
        root: usize,
        metric: Metric,
        matches: impl Fn(&Node) -> bool,
    ) -> Vec<(String, u64, u64)> {
        let mut map: HashMap<String, (u64, u64)> = HashMap::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            if node.directory {
                stack.extend(node.children.iter().copied());
                continue;
            }
            if !matches(node) {
                continue;
            }
            let ext = std::path::Path::new(&node.name)
                .extension()
                .map(|x| format!(".{}", x.to_string_lossy().to_lowercase()))
                .unwrap_or_else(|| "No extension".into());
            let entry = map.entry(ext).or_default();
            entry.0 = entry.0.saturating_add(node.size(metric));
            entry.1 += 1;
        }
        let mut items: Vec<_> = map
            .into_iter()
            .map(|(ext, (size, count))| (ext, size, count))
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.1));
        items
    }
}

pub fn bytes(value: u64) -> String {
    Units::default().format(value)
}

pub fn count(value: u64) -> String {
    let raw = value.to_string();
    raw.chars()
        .enumerate()
        .fold(String::new(), |mut out, (i, c)| {
            if i > 0 && (raw.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_remainder_requires_a_complete_matched_capture() {
        let mut scan = Scan {
            root: "C:\\".into(),
            nodes: vec![Node::new("C:".into(), None, true)],
            stats: ScanStats::default(),
        };
        scan.nodes[0].allocated = 800;
        assert_eq!(scan.outside_indexed(), None);
        scan.stats.volume_space = Some(VolumeSpace {
            capacity: 1000,
            free: 100,
            free_at_start: 110,
            captured_unix: 123,
        });
        assert_eq!(scan.outside_indexed(), Some(100));
        scan.stats.cancelled = true;
        assert_eq!(scan.outside_indexed(), None);
        scan.stats.cancelled = false;
        scan.nodes[0].allocated = 950;
        assert_eq!(scan.outside_indexed(), None);
        scan.nodes[0].allocated = 800;
        scan.nodes[0].allocation_unknown = true;
        assert_eq!(scan.outside_indexed(), None);
    }
    #[test]
    fn unit_labels_match_their_divisor() {
        assert_eq!(Units::Decimal.format(1_000_000_000), "1.00 GB");
        assert_eq!(Units::Binary.format(1_073_741_824), "1.00 GiB");
    }
    #[test]
    fn aggregate_arbitrary_order_and_shared_allocation() {
        let mut scan = Scan {
            root: "C:\\fixture".into(),
            stats: ScanStats::default(),
            nodes: vec![
                Node::new("fixture".into(), None, true),
                Node::new("one".into(), Some(2), false),
                Node::new("folder".into(), Some(0), true),
                Node::new("alias".into(), Some(0), false),
            ],
        };
        scan.nodes[0].children = vec![2, 3];
        scan.nodes[2].children = vec![1];
        scan.nodes[1].logical = 100;
        scan.nodes[1].allocated = 4096;
        scan.nodes[3].logical = 100;
        scan.nodes[3].shared = true;
        scan.finish();
        assert_eq!(
            (
                scan.nodes[0].logical,
                scan.nodes[0].allocated,
                scan.nodes[0].files
            ),
            (200, 4096, 2)
        );
        assert!(scan.path(1).ends_with("folder/one"));
    }
}
