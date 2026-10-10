use super::{Progress, windows};
use crate::model::{Node, Scan, ScanStats};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering, mpsc},
    time::Duration,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
};

struct Job {
    id: usize,
    path: PathBuf,
}
enum Message {
    Batch {
        parent: usize,
        entries: Vec<windows::DirectoryEntry>,
        queries: u64,
    },
    Done {
        id: usize,
        error: Option<std::io::Error>,
    },
}

pub fn scan_folder(root: PathBuf, progress: Arc<Progress>) -> Result<Scan, String> {
    let mut scan = Scan {
        root: windows::display_path(root.clone()),
        nodes: vec![Node::new(root.display().to_string(), None, true)],
        stats: ScanStats {
            engine: "Batched Windows folder scan".into(),
            ..Default::default()
        },
    };
    let (job_tx, job_rx) = mpsc::sync_channel::<Job>(8);
    let job_rx = Arc::new(Mutex::new(job_rx));
    let (tx, rx) = mpsc::sync_channel::<Message>(8);
    let mut pending = vec![0usize];
    let mut active = 0usize;
    let identities = Arc::new(Mutex::new(HashSet::new()));
    let mut unsupported = None;
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let job_rx = job_rx.clone();
            let tx = tx.clone();
            let progress = progress.clone();
            let identities = identities.clone();
            scope.spawn(move || {
                loop {
                    if progress.cancelled() {
                        break;
                    }
                    let job = match job_rx.lock().unwrap().recv() {
                        Ok(job) => job,
                        Err(_) => break,
                    };
                    if progress.cancelled() {
                        break;
                    }
                    let outcome = windows::enumerate_directory(&job.path, |mut entries, hint| {
                        if progress.cancelled() {
                            return false;
                        }
                        let mut queries = 0;
                        for entry in &mut entries {
                            if progress.cancelled() {
                                return false;
                            }
                            if entry.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                                continue;
                            }
                            if entry.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                                && std::fs::symlink_metadata(job.path.join(&entry.name))
                                    .is_ok_and(|m| m.is_symlink())
                            {
                                entry.symlink = true;
                                continue;
                            }
                            if entry.identity.is_some_and(|identity| {
                                !identities.lock().unwrap().insert(identity)
                            }) {
                                entry.shared = true;
                                entry.allocated = Some(0);
                                continue;
                            }
                            if entry.allocated != Some(0)
                                || entry.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                            {
                                entry.allocated =
                                    windows::allocation_from_entry(hint, &job.path, entry);
                                queries += 1;
                            }
                        }
                        tx.send(Message::Batch {
                            parent: job.id,
                            entries,
                            queries,
                        })
                        .is_ok()
                    });
                    if tx
                        .send(Message::Done {
                            id: job.id,
                            error: outcome.err(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        drop(tx);
        while (!pending.is_empty() || active > 0) && !progress.cancelled() {
            // Only IDs are queued in the coordinator; at most eight paths are in flight.
            while active < 8 && !pending.is_empty() {
                let id = pending.pop().unwrap();
                let path = if id == 0 { root.clone() } else { scan.path(id) };
                if job_tx.send(Job { id, path }).is_err() {
                    break;
                }
                active += 1;
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Message::Batch {
                    parent,
                    entries,
                    queries,
                }) => {
                    scan.stats.directory_batches += 1;
                    scan.stats.file_metadata_queries += queries;
                    for entry in entries {
                        if progress.cancelled() {
                            break;
                        }
                        let directory = entry.attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                        if entry.symlink
                            || (directory && entry.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0)
                        {
                            scan.stats.skipped_links += 1;
                            continue;
                        }
                        let mut node = Node::new(
                            entry.name.to_string_lossy().into_owned(),
                            Some(parent),
                            directory,
                        );
                        node.modified = entry.modified;
                        let id = scan.nodes.len();
                        if directory {
                            scan.stats.folders += 1;
                            progress.folders.fetch_add(1, Ordering::Relaxed);
                            pending.push(id);
                        } else {
                            scan.stats.files += 1;
                            progress.files.fetch_add(1, Ordering::Relaxed);
                            node.logical = entry.logical;
                            node.allocated = entry.allocated.unwrap_or(0);
                            node.allocation_unknown = entry.allocated.is_none();
                            scan.stats.unknown_allocations += u64::from(node.allocation_unknown);
                            node.shared = entry.shared;
                            scan.stats.shared_links += u64::from(entry.shared);
                            progress.bytes.fetch_add(node.allocated, Ordering::Relaxed);
                        }
                        scan.nodes.push(node);
                        scan.nodes[parent].children.push(id);
                    }
                }
                Ok(Message::Done { id, error }) => {
                    active -= 1;
                    if let Some(error) = error {
                        if id == 0 && matches!(error.raw_os_error(), Some(1 | 50 | 87)) {
                            unsupported = Some(error.to_string());
                            break;
                        }
                        scan.stats.errors += 1;
                        if scan.stats.notices.len() < 6 {
                            scan.stats
                                .notices
                                .push(format!("{}: {error}", scan.path(id).display()));
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(job_tx);
        drop(rx);
    });
    if let Some(reason) = unsupported {
        return Err(reason);
    }
    scan.stats.cancelled = progress.cancelled();
    scan.finish();
    Ok(scan)
}
