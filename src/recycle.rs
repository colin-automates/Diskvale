//! Single-target deletion. Each method requires its own explicit UI confirmation.
use crate::model::{RuntimeState, Scan};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Target {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
    pub logical: u64,
    pub allocated: u64,
    identity: (u64, u64),
    ancestors: Vec<(PathBuf, u64)>,
    authority: RuntimeState,
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
        if !scan.deletion_eligible() {
            return Err(
                "Deletion requires a fresh, complete live NTFS scan. Rescan the drive.".into(),
            );
        }
        let node = scan
            .nodes
            .get(id)
            .ok_or("This item is no longer in the scan.")?;
        if id == 0 {
            return Err("Drive and scan roots cannot be deleted.".into());
        }
        if node.synthetic || node.reparse || node.allocation_unknown {
            return Err(
                "This target cannot be safely verified. Rescan the original location.".into(),
            );
        }
        let identity = (
            scan.stats
                .volume_serial
                .ok_or("Missing volume identity. Rescan.")?,
            node.reference.ok_or("Missing NTFS identity. Rescan.")?,
        );
        let mut ancestors = vec![];
        let mut parts = vec![];
        let mut current = id;
        for _ in 0..scan.nodes.len() {
            if current == 0 {
                break;
            }
            let ancestor = scan.nodes.get(current).ok_or("Invalid scan path.")?;
            if ancestor.reparse || ancestor.synthetic {
                return Err(
                    "Deletion through links or junctions is unsafe. Rescan the original location."
                        .into(),
                );
            }
            if current != id {
                if !ancestor.directory {
                    return Err("Invalid non-directory ancestor. Rescan.".into());
                }
                ancestors.push((
                    PathBuf::new(),
                    ancestor
                        .reference
                        .ok_or("Missing ancestor identity. Rescan.")?,
                ));
            }
            if !safe_name(&ancestor.name) {
                return Err("This scan contains an unsafe file name. Rescan the drive.".into());
            }
            parts.push(ancestor.name.clone());
            current = ancestor.parent.ok_or("Invalid scan path.")?;
        }
        if current != 0 {
            return Err("Invalid scan path.".into());
        }
        ancestors.push((
            scan.root.clone(),
            scan.nodes[0]
                .reference
                .ok_or("Missing root identity. Rescan.")?,
        ));
        ancestors.reverse();
        let mut path = scan.root.clone();
        for (index, part) in parts.into_iter().rev().enumerate() {
            path.push(part);
            if let Some((ancestor_path, _)) = ancestors.get_mut(index + 1) {
                *ancestor_path = path.clone();
            }
        }
        validate_path(&path)?;
        Ok(Self {
            path,
            name: node.name.clone(),
            directory: node.directory,
            logical: node.logical,
            allocated: node.allocated,
            identity,
            ancestors,
            authority: scan.stats.runtime.clone(),
        })
    }
}

pub fn safe_name(name: &str) -> bool {
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
#[derive(Debug)]
struct TargetGuards {
    _ancestors: Vec<std::fs::File>,
    target: Option<std::fs::File>,
}

#[cfg(windows)]
fn check_live_target(target: &Target) -> Result<TargetGuards, String> {
    use crate::scanner::windows;
    use std::os::windows::fs::MetadataExt;
    validate_path(&target.path)?;
    if !target.authority.eligible() {
        return Err("This scan no longer authorizes deletion. Rescan.".into());
    }
    let mut guards = TargetGuards {
        _ancestors: vec![],
        target: None,
    };
    for (path, reference, is_target) in target
        .ancestors
        .iter()
        .map(|(p, r)| (p, *r, false))
        .chain(std::iter::once((&target.path, target.identity.1, true)))
    {
        // Ancestors deny DELETE sharing for the entire Shell operation. The target
        // permits Shell's rename/delete access but remains pinned against ID reuse.
        let handle = windows::identity_handle(path, is_target).map_err(|e| {
            format!(
                "Could not check {}: {e}. Rescan before deleting.",
                path.display()
            )
        })?;
        let live = windows::handle_identity(&handle)
            .map_err(|e| format!("Cannot verify NTFS identity: {e}. Rescan."))?;
        if live != (target.identity.0, reference) {
            return Err("This item or an ancestor was replaced, or its volume changed. Rescan before deleting.".into());
        }
        let metadata = handle
            .metadata()
            .map_err(|e| format!("Cannot verify target: {e}. Rescan."))?;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err("Delete is unavailable for links, junctions, or paths through them. Open the original location in Explorer.".into());
        }
        if is_target && metadata.is_dir() != target.directory {
            return Err("This item changed since the scan. Rescan before deleting it.".into());
        }
        if is_target && !target.directory && metadata.len() != target.logical {
            return Err(
                "This file's size changed since the scan. Rescan before deleting it.".into(),
            );
        }
        if !is_target && !metadata.is_dir() {
            return Err("Ancestor changed kind. Rescan.".into());
        }
        if is_target {
            guards.target = Some(handle);
        } else {
            guards._ancestors.push(handle);
        }
    }
    Ok(guards)
}

/// Called only after confirmation of this exact target and deletion method.
/// A recycling refusal is returned to the UI; it never retries permanently.
pub fn delete(target: &Target, method: Method) -> Result<Outcome, String> {
    #[cfg(windows)]
    {
        execute_verified(
            target,
            || {},
            |release_target| windows_backend::delete(target, method, release_target),
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (target, method);
        Err("Delete actions require Windows.".into())
    }
}

#[cfg(windows)]
fn execute_verified(
    target: &Target,
    pre_operation: impl FnOnce(),
    operation: impl FnOnce(&mut dyn FnMut()) -> Result<Outcome, String>,
) -> Result<Outcome, String> {
    let mut guards = check_live_target(target)?;
    pre_operation();
    let mut rechecked = check_live_target(target)?;
    let result = operation(&mut || {
        // Only after Shell finishes: Windows can defer permanent removal until
        // the last target handle closes. Ancestor pins remain held to return.
        drop(guards.target.take());
        drop(rechecked.target.take());
    });
    // Revoke at the helper boundary too: callers outside the UI must not reuse
    // authority after a successful or potentially partial backend operation.
    if result != Ok(Outcome::CannotRecycle) {
        target.authority.mark_stale();
    }
    result
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
        target: Option<Target>,
        invalid: Arc<AtomicBool>,
    }

    // In RecycleBin mode, veto Shell's proposed permanent deletion before it
    // starts. Permanent mode is a separate, explicitly confirmed operation.
    impl IFileOperationProgressSink_Impl for DeleteSink_Impl {
        fn PreDeleteItem(&self, flags: u32, _item: Ref<'_, IShellItem>) -> Result<()> {
            if let Some(target) = &self.target
                && check_live_target(target).is_err()
            {
                self.invalid.store(true, Ordering::Relaxed);
                return Err(Error::from_hresult(E_ABORT));
            }
            #[cfg(test)]
            eprintln!("Delete {:?} pre-delete flags: {flags:#X}", self.method);
            if self.method == Method::RecycleBin
                && flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0
            {
                if !self.completed.load(Ordering::Relaxed) {
                    self.refused.store(true, Ordering::Relaxed);
                }
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

    pub(super) fn delete(
        target: &Target,
        method: Method,
        release_target: impl FnOnce(),
    ) -> std::result::Result<Outcome, String> {
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
        let invalid = Arc::new(AtomicBool::new(false));
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
                    target: Some(target.clone()),
                    invalid: invalid.clone(),
                }
                .into();
                shell.DeleteItem(&item, &sink)?;
                shell.PerformOperations()?;
                Ok(shell.GetAnyOperationsAborted()?.as_bool())
            }
        })();
        if invalid.load(Ordering::Relaxed) {
            return Err(
                "Target changed before the Shell operation. Rescan before deleting.".into(),
            );
        }
        if refused.load(Ordering::Relaxed) && !completed.load(Ordering::Relaxed) {
            return Ok(Outcome::CannotRecycle);
        }
        let description = match method {
            Method::RecycleBin => "move this item to the Recycle Bin",
            Method::Permanent => "permanently delete this item",
        };
        let aborted = operation.map_err(|e| format!("Could not {description}: {e}"))?;
        if aborted || !completed.load(Ordering::Relaxed) {
            return Err(format!(
                "Windows did not finish trying to {description}. Rescan to check the current files."
            ));
        }
        release_target();
        let source_gone = std::fs::symlink_metadata(&target.path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
        if !source_gone {
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
                target: None,
                invalid: Arc::default(),
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
                target: None,
                invalid: Arc::default(),
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
                target: None,
                invalid: Arc::default(),
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
    #[cfg(windows)]
    fn fixture_scan(root: &std::path::Path) -> Scan {
        let mut scan = Scan {
            root: root.into(),
            stats: ScanStats::default(),
            nodes: vec![Node::new("fixture".into(), None, true)],
        };
        for entry in std::fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            let id = scan.nodes.len();
            let mut node = Node::new(
                entry.file_name().to_str().unwrap().into(),
                Some(0),
                entry.file_type().unwrap().is_dir(),
            );
            if !node.directory {
                node.logical = entry.metadata().unwrap().len();
            }
            scan.nodes.push(node);
            scan.nodes[0].children.push(id);
        }
        scan.stats.runtime = RuntimeState::new(crate::model::Provenance::Live);
        for id in 0..scan.nodes.len() {
            let handle = crate::scanner::windows::identity_handle(&scan.path(id), true).unwrap();
            let (volume, reference) = crate::scanner::windows::handle_identity(&handle).unwrap();
            scan.stats.directory_streams_captured = true;
            scan.stats.volume_serial = Some(volume);
            scan.nodes[id].reference = Some(reference);
        }
        scan.finish();
        scan
    }
    #[cfg(windows)]
    #[test]
    fn identity_rejects_same_size_replacement_directory_and_volume_and_keeps_contents_changes() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("file.txt"), b"same").unwrap();
        std::fs::create_dir(fixture.path().join("folder")).unwrap();
        let scan = fixture_scan(fixture.path());
        let target = |name| {
            Target::from_scan(
                &scan,
                scan.nodes.iter().position(|n| n.name == name).unwrap(),
            )
            .unwrap()
        };
        let file = target("file.txt");
        let folder = target("folder");
        assert!(check_live_target(&file).is_ok());
        assert!(check_live_target(&folder).is_ok());
        std::fs::write(
            folder.path.join("added-after-scan.txt"),
            b"permitted new contents",
        )
        .unwrap();
        assert!(check_live_target(&folder).is_ok());
        let mut wrong_volume = file.clone();
        wrong_volume.identity.0 ^= 1;
        assert!(
            check_live_target(&wrong_volume)
                .unwrap_err()
                .contains("volume changed")
        );
        std::fs::rename(&file.path, fixture.path().join("original-file.txt")).unwrap();
        std::fs::write(&file.path, b"same").unwrap();
        assert!(check_live_target(&file).unwrap_err().contains("replaced"));
        std::fs::rename(&folder.path, fixture.path().join("original-folder")).unwrap();
        std::fs::create_dir(&folder.path).unwrap();
        assert!(check_live_target(&folder).unwrap_err().contains("replaced"));
    }
    #[cfg(windows)]
    #[test]
    fn deterministic_pre_operation_replacement_is_refused_and_ancestors_are_pinned() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("file.txt"), b"same").unwrap();
        let scan = fixture_scan(fixture.path());
        let target = Target::from_scan(&scan, 1).unwrap();
        let result = execute_verified(
            &target,
            || {
                std::fs::rename(&target.path, fixture.path().join("original.txt")).unwrap();
                std::fs::write(&target.path, b"same").unwrap();
            },
            |_| panic!("Unsafe replacement reached the backend"),
        );
        assert!(result.unwrap_err().contains("replaced"));
        let scan = fixture_scan(fixture.path());
        let target = Target::from_scan(&scan, 1).unwrap();
        execute_verified(
            &target,
            || {
                assert!(
                    std::fs::rename(fixture.path(), fixture.path().with_extension("replaced"))
                        .is_err(),
                    "Ancestor guard permitted replacement"
                );
            },
            |_| Ok(Outcome::CannotRecycle),
        )
        .unwrap();
        assert!(target.path.exists());
    }
    #[cfg(windows)]
    #[test]
    fn serialized_flags_and_direct_helpers_cannot_mint_or_reuse_live_authority() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("file.txt"), b"same").unwrap();
        let mut scan = fixture_scan(fixture.path());
        let target = Target::from_scan(&scan, 1).unwrap();
        let mut json = serde_json::to_value(&scan).unwrap();
        json["stats"]["runtime"] = serde_json::json!({"provenance":"Live","stale":false});
        json["stats"]["notices"] = serde_json::json!(["Fresh live scan", "Experimental NTFS MFT"]);
        let saved: Scan = serde_json::from_value(json).unwrap();
        assert!(!saved.deletion_eligible());
        assert!(Target::from_scan(&saved, 1).is_err());
        scan.stats.cancelled = true;
        assert!(Target::from_scan(&scan, 1).is_err());
        scan.stats.cancelled = false;
        scan.stats.runtime.mark_stale();
        assert!(Target::from_scan(&scan, 1).is_err());
        assert!(
            execute_verified(&target, || {}, |_| panic!("Stale target reached backend")).is_err()
        );
        assert!(target.path.exists());
    }
    #[cfg(windows)]
    #[test]
    fn helper_revokes_all_retained_targets_after_operation_but_preserves_recycle_veto() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("file.txt"), b"same").unwrap();
        let scan = fixture_scan(fixture.path());
        let target = Target::from_scan(&scan, 1).unwrap();
        // Inject outcomes only; this test never calls Shell or deletes a file.
        assert_eq!(
            execute_verified(&target, || {}, |_| Ok(Outcome::CannotRecycle)),
            Ok(Outcome::CannotRecycle)
        );
        assert!(scan.deletion_eligible());
        assert_eq!(
            execute_verified(&target, || {}, |_| Ok(Outcome::Deleted)),
            Ok(Outcome::Deleted)
        );
        assert!(!scan.deletion_eligible());
        assert!(Target::from_scan(&scan, 1).is_err());
        assert!(
            execute_verified(&target, || {}, |_| panic!("Revoked target reached backend")).is_err()
        );
        assert_eq!(std::fs::read(&target.path).unwrap(), b"same");

        let scan = fixture_scan(fixture.path());
        let target = Target::from_scan(&scan, 1).unwrap();
        assert!(
            execute_verified(
                &target,
                || {},
                |_| Err("injected potentially partial backend failure".into())
            )
            .is_err()
        );
        assert!(!scan.deletion_eligible());
    }
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
        scan.stats.runtime = RuntimeState::new(crate::model::Provenance::Live);
        scan.stats.directory_streams_captured = true;
        scan.stats.volume_serial = Some(123);
        for (i, node) in scan.nodes.iter_mut().enumerate() {
            node.reference = Some((1 << 48) | i as u64);
        }
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
        scan.nodes.push(Node::new("cycle".into(), Some(1), true));
        scan.nodes[2].reference = Some((1 << 48) | 2);
        scan.nodes[1].parent = Some(2);
        assert!(Target::from_scan(&scan, 1).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn completion_keeps_ancestor_pins_until_after_target_release() {
        use std::os::windows::ffi::OsStrExt;
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(workspace.join("artifacts")).unwrap();
        let fixture = tempfile::Builder::new()
            .prefix("diskvale-completion-fixture-")
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
        std::fs::write(&path, b"disposable completion fixture").unwrap();
        let scan = fixture_scan(fixture.path());
        let target = Target::from_scan(&scan, 1).unwrap();
        assert_eq!(target.path, path);
        execute_verified(
            &target,
            || {},
            |release_target| {
                let wide: Vec<u16> = target
                    .path
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect();
                // Only this newly created fixture is selected for removal. Its parent
                // must stay pinned both during the operation and after target release.
                assert_ne!(
                    unsafe { windows_sys::Win32::Storage::FileSystem::DeleteFileW(wide.as_ptr()) },
                    0
                );
                assert!(
                    std::fs::rename(fixture.path(), fixture.path().with_extension("moved"))
                        .is_err()
                );
                release_target();
                assert_eq!(
                    std::fs::symlink_metadata(&path).unwrap_err().kind(),
                    std::io::ErrorKind::NotFound
                );
                assert!(
                    std::fs::rename(fixture.path(), fixture.path().with_extension("moved"))
                        .is_err()
                );
                Ok(Outcome::Deleted)
            },
        )
        .unwrap();
        assert!(!scan.deletion_eligible());
        let moved = fixture.path().with_extension("moved");
        assert!(!moved.exists());
        assert!(
            moved
                .parent()
                .unwrap()
                .canonicalize()
                .unwrap()
                .starts_with(workspace.canonicalize().unwrap())
        );
        std::fs::rename(fixture.path(), &moved).unwrap();
        std::fs::rename(&moved, fixture.path()).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn both_delete_methods_reject_roots_changed_kinds_sizes_and_missing_targets() {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(workspace.join("artifacts")).unwrap();
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
            identity: crate::scanner::windows::handle_identity(
                &crate::scanner::windows::identity_handle(&path, true).unwrap(),
            )
            .unwrap(),
            ancestors: vec![],
            authority: RuntimeState::new(crate::model::Provenance::Live),
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
        assert_eq!(
            std::env::var("DISKVALE_DELETE_FIXTURE_TESTS").as_deref(),
            Ok("1"),
            "Separate explicit fixture-test opt-in is required"
        );
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(workspace.join("artifacts")).unwrap();
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
        for name in ["canopy-test-file.txt", "canopy-test-folder"] {
            let scan = fixture_scan(fixture.path());
            let id = scan.nodes.iter().position(|n| n.name == name).unwrap();
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
        assert_eq!(
            std::env::var("DISKVALE_DELETE_FIXTURE_TESTS").as_deref(),
            Ok("1"),
            "Separate explicit fixture-test opt-in is required"
        );
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(workspace.join("artifacts")).unwrap();
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
        for name in ["canopy-test-file.txt", "canopy-test-folder"] {
            let scan = fixture_scan(fixture.path());
            let id = scan.nodes.iter().position(|n| n.name == name).unwrap();
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
