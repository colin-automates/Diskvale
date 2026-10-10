//! Single-target deletion. Each method requires its own explicit UI confirmation.
use crate::model::Scan;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Target {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
    pub logical: u64,
    pub allocated: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    RecycleBin,
    Permanent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Deleted,
    /// The recycle-only callback stopped Windows before permanent deletion.
    CannotRecycle,
}

impl Target {
    pub fn from_scan(scan: &Scan, id: usize) -> Result<Self, String> {
        let node = scan
            .nodes
            .get(id)
            .ok_or("This item is no longer in the scan.")?;
        if id == 0 {
            return Err("Drive and scan roots cannot be deleted.".into());
        }
        let mut parts = vec![];
        let mut current = id;
        for _ in 0..scan.nodes.len() {
            if current == 0 {
                break;
            }
            let ancestor = scan.nodes.get(current).ok_or("Invalid scan path.")?;
            if !safe_name(&ancestor.name) {
                return Err("This scan contains an unsafe file name. Rescan the drive.".into());
            }
            parts.push(ancestor.name.clone());
            current = ancestor.parent.ok_or("Invalid scan path.")?;
        }
        if current != 0 {
            return Err("Invalid scan path.".into());
        }
        let mut path = scan.root.clone();
        for part in parts.into_iter().rev() {
            path.push(part);
        }
        validate_path(&path)?;
        Ok(Self {
            path,
            name: node.name.clone(),
            directory: node.directory,
            logical: node.logical,
            allocated: node.allocated,
        })
    }
}

fn safe_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        });
    !name.is_empty()
        && !reserved
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
}

fn validate_path(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = path.components();
        if !matches!(components.next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
            || components.next() != Some(Component::RootDir)
        {
            return Err("Delete requires an absolute path on a local drive.".into());
        }
        let remaining: Vec<_> = components.collect();
        if remaining.is_empty()
            || remaining.iter().any(
                |c| !matches!(c, Component::Normal(name) if name.to_str().is_some_and(safe_name)),
            )
        {
            return Err("Drive roots and unsafe paths cannot be deleted.".into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("Delete actions require Windows.".into())
    }
}

#[cfg(windows)]
fn check_live_target(target: &Target) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    validate_path(&target.path)?;
    let mut parent = target.path.as_path();
    loop {
        let metadata = std::fs::symlink_metadata(parent).map_err(|e| {
            format!(
                "Could not check {}: {e}. Rescan if it was moved or deleted.",
                parent.display()
            )
        })?;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err("Delete is unavailable for links, junctions, or paths through them. Open the original location in Explorer.".into());
        }
        if parent == target.path && metadata.is_dir() != target.directory {
            return Err("This item changed since the scan. Rescan before deleting it.".into());
        }
        if parent == target.path && !target.directory && metadata.len() != target.logical {
            return Err(
                "This file's size changed since the scan. Rescan before deleting it.".into(),
            );
        }
        match parent.parent() {
            Some(next) => parent = next,
            None => break,
        }
    }
    Ok(())
}

/// Called only after confirmation of this exact target and deletion method.
/// A recycling refusal is returned to the UI; it never retries permanently.
pub fn delete(target: &Target, method: Method) -> Result<Outcome, String> {
    #[cfg(windows)]
    {
        check_live_target(target)?;
        windows_backend::delete(target, method)
    }
    #[cfg(not(windows))]
    {
        let _ = (target, method);
        Err("Delete actions require Windows.".into())
    }
}

#[cfg(windows)]
mod windows_backend {
    use super::*;
    use std::{
        os::windows::ffi::OsStrExt,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    use windows::{
        Win32::{
            Foundation::E_ABORT,
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            UI::Shell::*,
        },
        core::{Error, HRESULT, PCWSTR, Ref, Result, implement},
    };

    #[implement(IFileOperationProgressSink)]
    struct DeleteSink {
        method: Method,
        refused: Arc<AtomicBool>,
        completed: Arc<AtomicBool>,
    }

    // In RecycleBin mode, veto Shell's proposed permanent deletion before it
    // starts. Permanent mode is a separate, explicitly confirmed operation.
    impl IFileOperationProgressSink_Impl for DeleteSink_Impl {
        fn PreDeleteItem(&self, flags: u32, _item: Ref<'_, IShellItem>) -> Result<()> {
            #[cfg(test)]
            eprintln!("Delete {:?} pre-delete flags: {flags:#X}", self.method);
            if self.method == Method::RecycleBin
                && flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0
            {
                self.refused.store(true, Ordering::Relaxed);
                return Err(Error::from_hresult(E_ABORT));
            }
            Ok(())
        }
        fn PostDeleteItem(
            &self,
            _flags: u32,
            _item: Ref<'_, IShellItem>,
            result: HRESULT,
            recycled: Ref<'_, IShellItem>,
        ) -> Result<()> {
            #[cfg(test)]
            eprintln!(
                "Delete {:?} post-delete result: {:#X}, bin item: {}",
                self.method,
                result.0,
                !recycled.is_null()
            );
            // The returned Bin item distinguishes recycling from permanent
            // removal. Successful deletion also requires the source to be gone.
            let expected = match self.method {
                Method::RecycleBin => !recycled.is_null(),
                Method::Permanent => recycled.is_null(),
            };
            if result.is_ok() && expected {
                self.completed.store(true, Ordering::Relaxed);
            }
            result.ok()
        }
        fn StartOperations(&self) -> Result<()> {
            Ok(())
        }
        fn FinishOperations(&self, result: HRESULT) -> Result<()> {
            result.ok()
        }
        fn UpdateProgress(&self, _total: u32, _done: u32) -> Result<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> Result<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> Result<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> Result<()> {
            Ok(())
        }
        fn PreRenameItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR) -> Result<()> {
            Err(Error::from_hresult(E_ABORT))
        }
        fn PostRenameItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            hr: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> Result<()> {
            hr.ok()
        }
        fn PreMoveItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> Result<()> {
            Err(Error::from_hresult(E_ABORT))
        }
        fn PostMoveItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            hr: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> Result<()> {
            hr.ok()
        }
        fn PreCopyItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> Result<()> {
            Err(Error::from_hresult(E_ABORT))
        }
        fn PostCopyItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            hr: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> Result<()> {
            hr.ok()
        }
        fn PreNewItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR) -> Result<()> {
            Err(Error::from_hresult(E_ABORT))
        }
        fn PostNewItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: &PCWSTR,
            _: u32,
            hr: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> Result<()> {
            hr.ok()
        }
    }

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn operation_flags(method: Method) -> FILEOPERATION_FLAGS {
        let common = FOFX_EARLYFAILURE
            | FOF_NOERRORUI
            | FOF_SILENT
            | FOF_NOCONFIRMATION
            | FOF_NO_CONNECTED_ELEMENTS;
        match method {
            Method::RecycleBin => common | FOFX_RECYCLEONDELETE | FOFX_ADDUNDORECORD,
            Method::Permanent => common,
        }
    }

    pub(super) fn delete(target: &Target, method: Method) -> std::result::Result<Outcome, String> {
        let wide: Vec<u16> = target
            .path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }
            .map_err(|e| format!("Could not start the delete operation: {e}"))?;
        let _apartment = Apartment;
        let refused = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let operation = (|| -> Result<bool> {
            unsafe {
                let shell: IFileOperation =
                    CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
                shell.SetOperationFlags(operation_flags(method))?;
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)?;
                let sink: IFileOperationProgressSink = DeleteSink {
                    method,
                    refused: refused.clone(),
                    completed: completed.clone(),
                }
                .into();
                shell.DeleteItem(&item, &sink)?;
                shell.PerformOperations()?;
                Ok(shell.GetAnyOperationsAborted()?.as_bool())
            }
        })();
        if refused.load(Ordering::Relaxed) && !completed.load(Ordering::Relaxed) {
            return Ok(Outcome::CannotRecycle);
        }
        let description = match method {
            Method::RecycleBin => "move this item to the Recycle Bin",
            Method::Permanent => "permanently delete this item",
        };
        let aborted = operation.map_err(|e| format!("Could not {description}: {e}"))?;
        let source_gone = std::fs::symlink_metadata(&target.path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
        if aborted || !completed.load(Ordering::Relaxed) || !source_gone {
            return Err(format!(
                "Windows did not finish trying to {description}. Rescan to check the current files."
            ));
        }
        Ok(Outcome::Deleted)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn native_callback_vetoes_permanent_delete_without_operating_on_files() {
            let refused = Arc::new(AtomicBool::new(false));
            let sink: IFileOperationProgressSink = DeleteSink {
                method: Method::RecycleBin,
                refused: refused.clone(),
                completed: Arc::new(AtomicBool::new(false)),
            }
            .into();
            assert!(unsafe { sink.PreDeleteItem(0, None) }.is_err());
            assert!(refused.load(Ordering::Relaxed));
            assert!(
                unsafe { sink.PreDeleteItem(TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32, None) }
                    .is_ok()
            );
        }

        #[test]
        fn permanent_method_uses_no_recycle_or_undo_flags_and_accepts_full_delete() {
            let flags = operation_flags(Method::Permanent).0;
            assert_eq!(
                flags & (FOFX_RECYCLEONDELETE | FOFX_ADDUNDORECORD | FOF_ALLOWUNDO).0,
                0
            );
            assert_ne!(
                operation_flags(Method::RecycleBin).0 & FOFX_RECYCLEONDELETE.0,
                0
            );
            assert_eq!(flags & FOFX_NOSKIPJUNCTIONS.0, 0);
            let refused = Arc::new(AtomicBool::new(false));
            let completed = Arc::new(AtomicBool::new(false));
            let sink: IFileOperationProgressSink = DeleteSink {
                method: Method::Permanent,
                refused: refused.clone(),
                completed: completed.clone(),
            }
            .into();
            assert!(unsafe { sink.PreDeleteItem(0, None) }.is_ok());
            assert!(!refused.load(Ordering::Relaxed));
            assert!(unsafe { sink.PostDeleteItem(0, None, HRESULT(0), None) }.is_ok());
            assert!(completed.load(Ordering::Relaxed));
            completed.store(false, Ordering::Relaxed);
            assert!(unsafe { sink.PostDeleteItem(0, None, E_ABORT, None) }.is_err());
            assert!(!completed.load(Ordering::Relaxed));
        }

        #[test]
        fn recycle_callback_requires_a_bin_item_to_report_success() {
            let completed = Arc::new(AtomicBool::new(false));
            let sink: IFileOperationProgressSink = DeleteSink {
                method: Method::RecycleBin,
                refused: Arc::new(AtomicBool::new(false)),
                completed: completed.clone(),
            }
            .into();
            assert!(unsafe { sink.PostDeleteItem(0, None, HRESULT(0), None) }.is_ok());
            assert!(!completed.load(Ordering::Relaxed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Node, ScanStats};
    #[test]
    fn targets_are_literal_children_and_roots_are_blocked() {
        let mut scan = Scan {
            root: "C:\\".into(),
            nodes: vec![
                Node::new("C:\\".into(), None, true),
                Node::new("example.txt".into(), Some(0), false),
            ],
            stats: ScanStats::default(),
        };
        assert!(Target::from_scan(&scan, 0).is_err());
        #[cfg(windows)]
        assert_eq!(
            Target::from_scan(&scan, 1).unwrap().path,
            PathBuf::from("C:\\example.txt")
        );
        for name in [
            "..",
            "..\\Windows",
            "C:\\Windows",
            "file:stream",
            "*",
            "file.",
            "nul.txt",
            "COM1",
        ] {
            scan.nodes[1].name = name.into();
            assert!(Target::from_scan(&scan, 1).is_err(), "Accepted {name}");
        }
        scan.nodes[1].name = "example.txt".into();
        scan.nodes[1].parent = Some(1);
        assert!(Target::from_scan(&scan, 1).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn both_delete_methods_reject_roots_changed_kinds_sizes_and_missing_targets() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let fixture = tempfile::Builder::new()
            .prefix("canopy-live-guard-")
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
        std::fs::write(&path, b"unchanged fixture").unwrap();
        let target = Target {
            path: path.clone(),
            name: "new-disposable-file.txt".into(),
            directory: false,
            logical: 17,
            allocated: 17,
        };
        for method in [Method::RecycleBin, Method::Permanent] {
            let mut invalid = target.clone();
            invalid.path = PathBuf::from("C:\\");
            assert!(delete(&invalid, method).unwrap_err().contains("roots"));
            invalid = target.clone();
            invalid.directory = true;
            assert!(delete(&invalid, method).unwrap_err().contains("changed"));
            invalid = target.clone();
            invalid.logical += 1;
            assert!(
                delete(&invalid, method)
                    .unwrap_err()
                    .contains("size changed")
            );
            invalid = target.clone();
            invalid.path = fixture.path().join("not-created.txt");
            assert!(
                delete(&invalid, method)
                    .unwrap_err()
                    .contains("Could not check")
            );
        }
        assert_eq!(std::fs::read(path).unwrap(), b"unchanged fixture");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Explicit permanent-delete integration on new disposable workspace fixtures only"]
    fn shell_permanently_deletes_only_new_disposable_file_and_folder_fixtures() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let fixture = tempfile::Builder::new()
            .prefix("canopy-permanent-fixture-")
            .tempdir_in(workspace.join("artifacts"))
            .unwrap();
        assert!(
            fixture
                .path()
                .canonicalize()
                .unwrap()
                .starts_with(workspace.canonicalize().unwrap())
        );
        let outside = fixture.path().join("unselected-sibling.txt");
        std::fs::write(&outside, b"Unselected disposable sibling must survive").unwrap();
        std::fs::write(
            fixture.path().join("canopy-test-file.txt"),
            b"Disposable file",
        )
        .unwrap();
        std::fs::create_dir_all(fixture.path().join("canopy-test-folder/nested")).unwrap();
        std::fs::write(
            fixture.path().join("canopy-test-folder/nested/child.txt"),
            b"Disposable nested child",
        )
        .unwrap();
        let mut scan = Scan {
            root: fixture.path().into(),
            nodes: vec![
                Node::new("fixture".into(), None, true),
                Node::new("canopy-test-file.txt".into(), Some(0), false),
                Node::new("canopy-test-folder".into(), Some(0), true),
            ],
            stats: ScanStats::default(),
        };
        scan.nodes[1].logical = 15;
        for id in [1, 2] {
            let target = Target::from_scan(&scan, id).unwrap();
            assert!(target.path.starts_with(fixture.path()));
            assert!(
                target
                    .path
                    .canonicalize()
                    .unwrap()
                    .starts_with(fixture.path().canonicalize().unwrap())
            );
            assert_eq!(
                delete(&target, Method::Permanent).unwrap(),
                Outcome::Deleted
            );
            assert_eq!(
                std::fs::symlink_metadata(&target.path).unwrap_err().kind(),
                std::io::ErrorKind::NotFound
            );
            assert_eq!(
                std::fs::read(&outside).unwrap(),
                b"Unselected disposable sibling must survive"
            );
        }
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "Explicit disposable-fixture Recycle Bin integration test"]
    fn shell_recycles_only_new_disposable_file_and_folder_fixtures() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let resolved_workspace = workspace.canonicalize().unwrap();
        let fixture = tempfile::Builder::new()
            .prefix("canopy-recycle-fixture-")
            .tempdir_in(workspace.join("artifacts"))
            .unwrap();
        assert!(
            fixture
                .path()
                .canonicalize()
                .unwrap()
                .starts_with(&resolved_workspace)
        );
        std::fs::write(
            fixture.path().join("canopy-test-file.txt"),
            b"Canopy disposable recycling test",
        )
        .unwrap();
        std::fs::create_dir(fixture.path().join("canopy-test-folder")).unwrap();
        std::fs::write(
            fixture.path().join("canopy-test-folder/child.txt"),
            b"Disposable child",
        )
        .unwrap();
        let mut scan = Scan {
            root: fixture.path().into(),
            nodes: vec![
                Node::new("fixture".into(), None, true),
                Node::new("canopy-test-file.txt".into(), Some(0), false),
                Node::new("canopy-test-folder".into(), Some(0), true),
            ],
            stats: ScanStats::default(),
        };
        scan.nodes[1].logical = std::fs::metadata(fixture.path().join("canopy-test-file.txt"))
            .unwrap()
            .len();
        for id in [1, 2] {
            let target = Target::from_scan(&scan, id).unwrap();
            assert!(target.path.starts_with(fixture.path()));
            assert_eq!(
                delete(&target, Method::RecycleBin).unwrap(),
                Outcome::Deleted
            );
            assert_eq!(
                std::fs::symlink_metadata(&target.path).unwrap_err().kind(),
                std::io::ErrorKind::NotFound
            );
        }
    }
}
