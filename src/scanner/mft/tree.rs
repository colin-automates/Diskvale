use super::{
    IndexError,
    format::{self, Record},
};
use crate::{
    model::{Node, Scan, ScanStats},
    scanner::Progress,
};
use std::{collections::HashMap, path::Path};

pub(super) fn build(
    root: &Path,
    mut records: Vec<Record>,
    progress: &Progress,
    engine: &str,
) -> Result<Scan, IndexError> {
    let cancelled = progress.cancelled();
    let root_ref = records
        .iter()
        .find(|r| format::slot(r.reference) == 5 && r.directory)
        .map(|r| r.reference);
    let mut scan = Scan {
        root: root.to_path_buf(),
        nodes: vec![Node::new(root.display().to_string(), None, true)],
        stats: ScanStats {
            runtime: crate::model::RuntimeState::new(if cancelled {
                crate::model::Provenance::Incomplete
            } else {
                crate::model::Provenance::Snapshot
            }),
            schema_version: 2,
            directory_streams_captured: true,
            engine: engine.into(),
            cancelled,
            ..Default::default()
        },
    };
    if root_ref.is_none() && !cancelled {
        return Err("Missing NTFS root record".into());
    }
    let Some(root_ref) = root_ref else {
        scan.stats
            .notices
            .push("Cancelled before the NTFS root was read; the partial tree is empty.".into());
        return Ok(scan);
    };
    for record in &mut records {
        record.normalize_names();
        record
            .finalize_streams(cancelled)
            .map_err(|e| format!("NTFS record {}: {e}", format::slot(record.reference)))?;
    }
    // Preserve directory-owned data separately from descendant totals. A synthetic
    // leaf supplies the existing map/query/type pipelines without inventing a path.
    fn directory_data(node: &mut Node, record: &Record) -> Result<(), String> {
        node.reference = Some(record.reference);
        node.reparse = record.reparse;
        node.modified = record.modified;
        node.own_logical = record
            .named_logical()?
            .checked_add(record.data.map_or(0, |s| s.0))
            .ok_or("Directory stream size overflow")?;
        node.own_allocated = record.allocation()?;
        node.allocation_unknown = record.incomplete;
        Ok(())
    }
    if let Some(record) = records.iter().find(|r| r.reference == root_ref) {
        directory_data(&mut scan.nodes[0], record)?;
    }
    // An active record with zero links need not belong to the directory namespace.
    // Keep its accounting explicit rather than inventing a filesystem path for it.
    for record in records.iter().filter(|r| r.names.is_empty()) {
        if record.links != 0 && !cancelled {
            return Err(format!(
                "NTFS record {} has {} links but no file name",
                format::slot(record.reference),
                record.links
            )
            .into());
        }
        scan.stats.unlinked_records += 1;
        scan.stats.unlinked_logical_bytes = scan
            .stats
            .unlinked_logical_bytes
            .checked_add(record.data.map_or(0, |s| s.0))
            .ok_or("Unlinked logical size overflow")?;
        scan.stats.unlinked_allocated_bytes = scan
            .stats
            .unlinked_allocated_bytes
            .checked_add(record.allocation()?)
            .ok_or("Unlinked allocation overflow")?;
    }
    records.retain(|r| !r.names.is_empty());
    // Reserve by normalized names, including hard links, with at most one
    // stream bucket per directory. This avoids copying a large node array.
    let node_capacity = records
        .iter()
        .try_fold(0usize, |sum, record| {
            sum.checked_add(record.names.len())
                .and_then(|n| n.checked_add(usize::from(record.directory)))
        })
        .ok_or("NTFS node count overflow")?;
    scan.nodes
        .try_reserve_exact(node_capacity.saturating_sub(1))
        .map_err(|_| "Insufficient memory for the NTFS tree")?;
    let mut dirs = HashMap::from([(root_ref, 0usize)]);
    let mut pending = vec![];
    for record in records
        .iter_mut()
        .filter(|r| r.directory && r.reference != root_ref)
    {
        if record.names.len() != 1 {
            if cancelled {
                continue;
            }
            return Err("Unsupported NTFS directory names".into());
        }
        let id = scan.nodes.len();
        let mut node = Node::new(std::mem::take(&mut record.names[0].text), None, true);
        directory_data(&mut node, record)?;
        scan.nodes.push(node);
        dirs.insert(record.reference, id);
        pending.push((id, record.reference, record.names[0].parent));
    }
    for (id, child_reference, parent) in pending {
        if let Some(&parent) = dirs.get(&parent) {
            if parent == id && !cancelled {
                return Err("Cyclic NTFS directory".into());
            }
            scan.nodes[id].parent = Some(parent);
            scan.nodes[parent].children.push(id);
        } else if !cancelled {
            let observed = records
                .iter()
                .find(|record| format::slot(record.reference) == format::slot(parent))
                .map(|record| (record.reference, record.directory, !record.names.is_empty()));
            return Err(parent_reference_error(
                "directory",
                child_reference,
                parent,
                observed,
            ));
        }
    }
    let mut seen = vec![false; scan.nodes.len()];
    let mut stack = vec![0];
    while let Some(id) = stack.pop() {
        if seen[id] {
            return Err("Cyclic NTFS directory graph".into());
        }
        seen[id] = true;
        stack.extend(scan.nodes[id].children.iter().copied());
    }
    if !cancelled && seen.iter().any(|s| !*s) {
        return Err("Detached or cyclic NTFS directory graph".into());
    }
    if cancelled {
        let mut remap = vec![usize::MAX; scan.nodes.len()];
        let mut retained = vec![];
        for (id, node) in std::mem::take(&mut scan.nodes).into_iter().enumerate() {
            if seen[id] {
                remap[id] = retained.len();
                retained.push(node);
            } else {
                scan.stats.errors += 1;
            }
        }
        for node in &mut retained {
            node.parent = node.parent.map(|p| remap[p]);
            node.children.retain(|&id| seen[id]);
            for id in &mut node.children {
                *id = remap[*id];
            }
        }
        dirs.retain(|_, id| {
            let new = remap[*id];
            *id = new;
            new != usize::MAX
        });
        scan.nodes = retained;
    }
    scan.stats.folders = scan.nodes.len().saturating_sub(1) as u64;
    for id in 0..scan.nodes.len() {
        let owner = &scan.nodes[id];
        if owner.own_logical == 0 && owner.own_allocated == 0 && !owner.allocation_unknown {
            continue;
        }
        let mut bucket = Node::new("Directory streams".into(), Some(id), false);
        bucket.synthetic = true;
        bucket.files = 0;
        bucket.logical = owner.own_logical;
        bucket.allocated = owner.own_allocated;
        bucket.allocation_unknown = owner.allocation_unknown;
        bucket.modified = owner.modified;
        let child = scan.nodes.len();
        scan.nodes.push(bucket);
        scan.nodes[id].children.push(child);
    }
    for record in records.into_iter().filter(|r| !r.directory) {
        if !cancelled && progress.cancelled() {
            scan.stats.cancelled = true;
            break;
        }
        if record.names.is_empty() {
            if cancelled {
                scan.stats.errors += 1;
                continue;
            }
            return Err(format!(
                "NTFS record {} has no file name",
                format::slot(record.reference)
            )
            .into());
        }
        let allocated = record.allocation()?;
        let logical = record.data.map_or(0, |s| s.0);
        let incomplete = record.incomplete;
        let named_logical = record.named_logical()?;
        let mut linked = false;
        for name in record.names {
            let Some(&parent) = dirs.get(&name.parent) else {
                if cancelled {
                    scan.stats.errors += 1;
                    continue;
                }
                let observed = dirs
                    .keys()
                    .find(|&&reference| format::slot(reference) == format::slot(name.parent))
                    .map(|&reference| (reference, true, true));
                return Err(parent_reference_error(
                    "file",
                    record.reference,
                    name.parent,
                    observed,
                ));
            };
            let mut node = Node::new(name.text, Some(parent), false);
            node.logical = logical;
            node.named_logical = named_logical;
            node.reference = Some(record.reference);
            node.reparse = record.reparse;
            node.allocated = if linked { 0 } else { allocated };
            node.shared = linked;
            node.modified = record.modified;
            node.allocation_unknown = incomplete;
            if incomplete {
                scan.stats.unknown_allocations += 1;
            }
            if linked {
                scan.stats.shared_links += 1;
            }
            linked = true;
            let id = scan.nodes.len();
            scan.nodes.push(node);
            scan.nodes[parent].children.push(id);
            scan.stats.files += 1;
        }
    }
    scan.stats.cancelled |= progress.cancelled();
    if scan.stats.cancelled {
        scan.stats.runtime = crate::model::RuntimeState::new(crate::model::Provenance::Incomplete);
    }
    if scan.stats.unlinked_records > 0 {
        scan.stats.notices.push(format!("{} active NTFS records have no directory names and are excluded from the visible tree; their data totals {} logical and {} allocated bytes.",scan.stats.unlinked_records,scan.stats.unlinked_logical_bytes,scan.stats.unlinked_allocated_bytes));
    }
    scan.stats.notices.push("Experimental accounting includes named data streams and NTFS metadata files. Directory index allocation and volume overhead are not included; a live scan is not an atomic snapshot.".into());
    if scan.stats.cancelled {
        scan.stats.notices.push("Cancelled: only records with a complete parent chain are included in this partial result.".into());
    }
    scan.finish();
    Ok(scan)
}

fn parent_reference_error(
    kind: &str,
    child: u64,
    parent: u64,
    observed: Option<(u64, bool, bool)>,
) -> IndexError {
    let detail = match observed {
        Some((reference, directory, named)) => format!(
            "scanned parent sequence {} ({}; {})",
            reference >> 48,
            if directory {
                "directory"
            } else {
                "not a directory"
            },
            if named { "has a name" } else { "has no name" }
        ),
        None => "parent absent from the scanned named directories".into(),
    };
    // Record numbers/sequences are diagnostic identifiers; never include paths
    // or filenames in the error the user may share publicly.
    IndexError::ParentReference(format!(
        "NTFS {kind} parent is missing or reused (child record {}, sequence {}; expected parent record {}, sequence {}; {detail}); rescan the live volume",
        format::slot(child),
        child >> 48,
        format::slot(parent),
        parent >> 48
    ))
}
