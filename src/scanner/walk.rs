use super::Progress;
use crate::model::{Node, Scan, ScanStats};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::UNIX_EPOCH,
};

#[derive(Default, Clone, Copy, Debug)]
struct EntryData {
    logical: u64,
    allocated: Option<u64>,
    modified: u64,
    identity: Option<(u32, u64)>,
    links: u32,
    skip: bool,
    failed: bool,
}

pub fn scan_folder(root: PathBuf, progress: Arc<Progress>) -> Result<Scan, String> {
    #[cfg(windows)]
    let root = super::windows::display_path(root);
    let mut result = Scan {
        root: root.clone(),
        nodes: vec![Node::new(
            root.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string()),
            None,
            true,
        )],
        stats: ScanStats {
            engine: "Parallel folder scan".into(),
            ..Default::default()
        },
    };
    let mut dirs = HashMap::from([(root.clone(), 0usize)]);
    let mut identities = HashSet::new();
    let worker_progress = progress.clone();
    let walk = jwalk::WalkDirGeneric::<((), EntryData)>::new(&root)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(jwalk::Parallelism::RayonNewPool(4))
        .process_read_dir(move |_depth, _path, _state, children| {
            for entry in children.iter_mut().flatten() {
                if worker_progress.cancelled() {
                    entry.read_children_path = None;
                    continue;
                }
                let path = entry.path();
                let metadata = match std::fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(_) => {
                        entry.client_state.failed = true;
                        entry.read_children_path = None;
                        continue;
                    }
                };
                #[cfg(windows)]
                let attrs = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes()
                };
                #[cfg(not(windows))]
                let attrs = 0u32;
                let skip = entry.file_type.is_symlink()
                    || (entry.file_type.is_dir() && attrs & 0x400 != 0);
                if skip {
                    entry.read_children_path = None;
                    entry.client_state.skip = true;
                    continue;
                }
                entry.client_state.logical = if metadata.is_file() {
                    metadata.len()
                } else {
                    0
                };
                entry.client_state.modified = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if metadata.is_file() {
                    #[cfg(windows)]
                    {
                        let m = super::windows::measure(&path, attrs);
                        entry.client_state.allocated = m.allocated;
                        entry.client_state.identity = m.identity;
                        entry.client_state.links = m.links;
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        entry.client_state.allocated = Some(metadata.blocks() * 512);
                        entry.client_state.identity = Some((metadata.dev() as u32, metadata.ino()));
                        entry.client_state.links = metadata.nlink() as u32;
                    }
                }
            }
            if worker_progress.cancelled() {
                children.clear();
            }
        });
    for item in walk {
        if progress.cancelled() {
            result.stats.cancelled = true;
            break;
        }
        let entry = match item {
            Ok(e) => e,
            Err(e) => {
                result.stats.errors += 1;
                if result.stats.notices.len() < 6 {
                    result.stats.notices.push(e.to_string());
                }
                continue;
            }
        };
        if let Some(error) = &entry.read_children_error {
            result.stats.errors += 1;
            if result.stats.notices.len() < 6 {
                result.stats.notices.push(error.to_string());
            }
        }
        if entry.depth == 0 {
            continue;
        }
        let data = entry.client_state;
        if data.failed {
            result.stats.errors += 1;
            continue;
        }
        if data.skip {
            result.stats.skipped_links += 1;
            continue;
        }
        let path = entry.path();
        let Some(parent) = path.parent().and_then(|p| dirs.get(p)).copied() else {
            result.stats.errors += 1;
            continue;
        };
        let directory = entry.file_type.is_dir();
        let mut node = Node::new(
            entry.file_name.to_string_lossy().into_owned(),
            Some(parent),
            directory,
        );
        node.modified = data.modified;
        node.logical = data.logical;
        if directory {
            result.stats.folders += 1;
            progress.folders.fetch_add(1, Ordering::Relaxed);
        } else {
            result.stats.files += 1;
            progress.files.fetch_add(1, Ordering::Relaxed);
            node.allocation_unknown = data.allocated.is_none();
            if node.allocation_unknown {
                result.stats.unknown_allocations += 1;
            }
            node.allocated = data.allocated.unwrap_or(0);
            if data.links > 1 {
                if let Some(identity) = data.identity {
                    if !identities.insert(identity) {
                        node.allocated = 0;
                        node.shared = true;
                        result.stats.shared_links += 1;
                    }
                } else {
                    node.allocation_unknown = true;
                    result.stats.unknown_allocations += u64::from(data.allocated.is_some());
                }
            }
            progress.bytes.fetch_add(node.allocated, Ordering::Relaxed);
        }
        let id = result.nodes.len();
        result.nodes.push(node);
        result.nodes[parent].children.push(id);
        if directory {
            dirs.insert(path, id);
        }
    }
    result.stats.cancelled |= progress.cancelled();
    result.finish();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_files_hard_links_and_empty_folders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("empty")).unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("a.bin"), vec![7u8; 8192]).unwrap();
        std::fs::hard_link(dir.path().join("a.bin"), dir.path().join("nested/b.bin")).unwrap();
        std::fs::write(dir.path().join("small.txt"), b"hello").unwrap();
        let scan = scan_folder(dir.path().into(), Arc::new(Progress::default())).unwrap();
        assert_eq!(scan.stats.files, 3);
        assert_eq!(scan.stats.folders, 2);
        assert_eq!(scan.nodes[0].logical, 8192 * 2 + 5);
        assert_eq!(scan.stats.shared_links, 1);
        assert_eq!(scan.stats.unknown_allocations, 0);
        assert!(scan.nodes[0].allocated < 8192 * 2 + 5);
    }
    #[test]
    fn cancel_returns_an_explicit_partial_scan() {
        let dir = tempfile::tempdir().unwrap();
        let progress = Arc::new(Progress::default());
        progress.cancel.store(true, Ordering::Relaxed);
        let scan = scan_folder(dir.path().into(), progress).unwrap();
        assert!(scan.stats.cancelled);
    }
}
