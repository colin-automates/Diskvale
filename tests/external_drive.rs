//! Opt-in metadata scan of a named exFAT/FAT drive. No fixture or output is
//! written to the drive. Ordinary `cargo test` never scans live volumes.
#![cfg(windows)]

#[test]
#[ignore = "Explicit read-only external drive check; set DISKVALE_READ_ONLY_DRIVE to its root"]
fn explicit_external_drive_reads_metadata_without_deletion_authority() {
    use canopy::scanner::{self, Progress};
    use std::{
        sync::{Arc, atomic::Ordering, mpsc},
        time::Duration,
    };
    let root = std::env::var("DISKVALE_READ_ONLY_DRIVE").expect("Set an explicit whole drive root");
    let drive = scanner::windows::drive(std::path::Path::new(&root)).unwrap();
    assert_eq!(
        scanner::DriveScanMethod::for_filesystem(&drive.filesystem),
        Some(scanner::DriveScanMethod::Directory)
    );
    let progress = Arc::new(Progress::default());
    let (finished, receive_finished) = mpsc::channel();
    let cancel = progress.clone();
    let timer = std::thread::spawn(move || {
        if receive_finished
            .recv_timeout(Duration::from_secs(30))
            .is_err()
        {
            cancel.cancel.store(true, Ordering::Relaxed);
        }
    });
    let result = scanner::scan_drive(root.into(), progress);
    let _ = finished.send(());
    timer.join().unwrap();
    let scan = result.unwrap();
    assert_eq!(
        scan.stats.filesystem.as_deref(),
        Some(drive.filesystem.as_str())
    );
    assert!(!scan.deletion_eligible());
    assert!(!scan.nodes.is_empty());
    println!(
        "{}",
        serde_json::json!({"filesystem":scan.stats.filesystem,"engine":scan.stats.engine,
        "files":scan.stats.files,"folders":scan.stats.folders,"logical_bytes":scan.nodes[0].logical,
        "allocated_bytes":scan.nodes[0].allocated,"errors":scan.stats.errors,"unknown_allocations":scan.stats.unknown_allocations,
        "cancelled":scan.stats.cancelled,"elapsed_seconds":scan.stats.elapsed_seconds,"deletion_eligible":scan.deletion_eligible()})
    );
}
