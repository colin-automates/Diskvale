use canopy::{
    model::Metric,
    scanner::{self, Progress},
};
use std::{fs, sync::Arc};

#[test]
fn public_scanner_keeps_cancellation_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let progress = Arc::new(Progress::default());
    progress
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let scan = scanner::scan(dir.path().into(), progress, false).unwrap();
    assert!(scan.stats.cancelled);
}

#[test]
fn public_scanner_deduplicates_hard_links_across_folders() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    let file = dir.path().join("one.bin");
    fs::write(&file, vec![1u8; 16384]).unwrap();
    fs::hard_link(&file, dir.path().join("nested/two.bin")).unwrap();
    let scan = scanner::scan(dir.path().into(), Arc::new(Progress::default()), false).unwrap();
    assert_eq!(scan.stats.files, 2);
    assert_eq!(scan.stats.shared_links, 1);
    assert_eq!(scan.nodes[0].logical, 32768);
    assert_eq!(scan.nodes[0].allocated, 16384);
    assert_eq!(scan.stats.unknown_allocations, 0);
}

#[test]
fn scans_unicode_paths_and_scoped_extensions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("Renders – 日本語")).unwrap();
    fs::write(root.join("outside.zip"), vec![0u8; 8192]).unwrap();
    fs::write(
        root.join("Renders – 日本語/scene,final.png"),
        vec![0u8; 16384],
    )
    .unwrap();
    let scan = scanner::scan(root.into(), Arc::new(Progress::default()), false).unwrap();
    assert_eq!(scan.stats.errors, 0);
    assert_eq!(scan.stats.files, 2);
    assert_eq!(scan.nodes[0].logical, 24576);
    let folder = scan
        .nodes
        .iter()
        .position(|n| n.name == "Renders – 日本語")
        .unwrap();
    let types = scan.extensions_under(folder, Metric::Logical);
    assert_eq!(types, vec![(".png".into(), 16384, 1)]);
    let file = scan
        .nodes
        .iter()
        .position(|n| n.name == "scene,final.png")
        .unwrap();
    assert_eq!(fs::metadata(scan.path(file)).unwrap().len(), 16384);
}

#[test]
fn rejects_file_as_scan_target() {
    let file = tempfile::NamedTempFile::new().unwrap();
    assert!(
        scanner::scan(file.path().into(), Arc::new(Progress::default()), false)
            .unwrap_err()
            .contains("folder")
    );
}

#[cfg(windows)]
#[test]
fn distinguishes_resident_data_from_small_nonresident_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("resident.txt"), b"canopy resident data").unwrap();
    let grown = dir.path().join("nonresident.bin");
    fs::write(&grown, vec![1u8; 16384]).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&grown)
        .unwrap()
        .set_len(736)
        .unwrap();
    let scan = scanner::scan(dir.path().into(), Arc::new(Progress::default()), false).unwrap();
    let resident = scan
        .nodes
        .iter()
        .find(|node| node.name == "resident.txt")
        .unwrap();
    let nonresident = scan
        .nodes
        .iter()
        .find(|node| node.name == "nonresident.bin")
        .unwrap();
    assert_eq!(resident.logical, 20);
    assert_eq!(
        resident.allocated, 0,
        "Resident data occupies its existing NTFS record"
    );
    assert_eq!(nonresident.logical, 736);
    assert!(
        nonresident.allocated >= 4096,
        "A small file can still have physical extents"
    );
    assert_eq!(scan.stats.unknown_allocations, 0);
}

#[cfg(windows)]
#[test]
fn measures_files_beyond_legacy_windows_path_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut nested = dir.path().to_path_buf();
    for i in 0..8 {
        nested.push(format!("long-directory-{i}-abcdefghijklmnopqrstuvwxyz"));
    }
    fs::create_dir_all(&nested).unwrap();
    let path = nested.join("large.bin");
    assert!(path.as_os_str().len() > 260);
    fs::write(&path, vec![1u8; 16384]).unwrap();
    let scan = scanner::scan(dir.path().into(), Arc::new(Progress::default()), false).unwrap();
    assert_eq!(scan.stats.files, 1);
    assert_eq!(scan.stats.errors, 0);
    assert_eq!(scan.stats.unknown_allocations, 0);
    assert_eq!(scan.nodes[0].logical, 16384);
    assert_eq!(scan.nodes[0].allocated, 16384);
}

#[cfg(windows)]
#[test]
fn reports_sparse_physical_allocation_without_reading_contents() {
    use std::{
        io::{Seek, SeekFrom, Write},
        os::windows::io::AsRawHandle,
        ptr,
    };
    use windows_sys::Win32::System::{IO::DeviceIoControl, Ioctl::FSCTL_SET_SPARSE};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sparse.bin");
    let mut file = fs::File::create(&path).unwrap();
    let mut returned = 0;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            FSCTL_SET_SPARSE,
            ptr::null(),
            0,
            ptr::null_mut(),
            0,
            &mut returned,
            ptr::null_mut(),
        )
    };
    assert_ne!(ok, 0, "Temporary fixture must support NTFS sparse files");
    file.seek(SeekFrom::Start(64 * 1024 * 1024)).unwrap();
    file.write_all(b"canopy").unwrap();
    drop(file);
    let scan = scanner::scan(dir.path().into(), Arc::new(Progress::default()), false).unwrap();
    assert_eq!(scan.nodes[0].logical, 64 * 1024 * 1024 + 6);
    assert_eq!(scan.stats.unknown_allocations, 0);
    assert!(scan.nodes[0].allocated < 1024 * 1024);
}
