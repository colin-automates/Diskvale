use std::{
    ffi::OsString,
    fs::File,
    io,
    mem::{size_of, zeroed},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_HANDLE_EOF, ERROR_MORE_DATA, ERROR_NO_MORE_FILES, GetLastError,
        INVALID_HANDLE_VALUE, SetLastError,
    },
    Storage::FileSystem::*,
    System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE},
};

#[derive(Debug)]
pub struct DirectoryEntry {
    pub name: OsString,
    pub attributes: u32,
    pub logical: u64,
    pub allocated: Option<u64>,
    pub modified: u64,
    pub identity: Option<(u32, u64)>,
    pub shared: bool,
    pub symlink: bool,
}

pub(super) struct DirectoryHandle(windows_sys::Win32::Foundation::HANDLE);
impl Drop for DirectoryHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Buffered directory metadata through a normal directory handle, never a volume handle.
/// The callback runs once per batch and can stop enumeration without reading contents.
pub(super) fn enumerate_directory(
    path: &Path,
    mut batch: impl FnMut(Vec<DirectoryEntry>, &DirectoryHandle) -> bool,
) -> io::Result<()> {
    let w = wide(&extended_path(path));
    let handle = unsafe {
        CreateFileW(
            w.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let handle = DirectoryHandle(handle);
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let volume = if unsafe { GetFileInformationByHandle(handle.0, &mut info) } != 0 {
        Some(info.dwVolumeSerialNumber)
    } else {
        None
    };
    // u64 provides the 8-byte alignment required by FILE_ID_BOTH_DIR_INFO.
    let mut buffer = vec![0u64; 8192];
    let bytes = buffer.len() * size_of::<u64>();
    let header = std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let mut first = true;
    loop {
        buffer.fill(0);
        let class = if first {
            FileIdBothDirectoryRestartInfo
        } else {
            FileIdBothDirectoryInfo
        };
        first = false;
        let ok = unsafe {
            GetFileInformationByHandleEx(handle.0, class, buffer.as_mut_ptr() as _, bytes as u32)
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(());
            }
            return Err(error);
        }
        let mut entries = Vec::with_capacity(256);
        let mut offset = 0usize;
        loop {
            if offset + size_of::<FILE_ID_BOTH_DIR_INFO>() > bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Truncated directory record",
                ));
            }
            let entry = unsafe {
                ptr::read_unaligned(
                    (buffer.as_ptr() as *const u8).add(offset) as *const FILE_ID_BOTH_DIR_INFO
                )
            };
            let next = entry.NextEntryOffset as usize;
            let end = if next == 0 {
                bytes
            } else {
                offset
                    .checked_add(next)
                    .filter(|&end| end <= bytes)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "Invalid directory offset")
                    })?
            };
            let length = entry.FileNameLength as usize;
            if length == 0
                || !length.is_multiple_of(2)
                || offset + header + length > end
                || (next != 0 && (next < header || !next.is_multiple_of(8)))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid directory name bounds",
                ));
            }
            let name = unsafe {
                std::slice::from_raw_parts(
                    (buffer.as_ptr() as *const u8).add(offset + header) as *const u16,
                    length / 2,
                )
            };
            let name = OsString::from_wide(name);
            if name != "." && name != ".." {
                entries.push(DirectoryEntry {
                    name,
                    attributes: entry.FileAttributes,
                    logical: entry.EndOfFile.max(0) as u64,
                    allocated: (entry.AllocationSize >= 0).then_some(entry.AllocationSize as u64),
                    modified: (entry.LastWriteTime.max(0) as u64)
                        .saturating_sub(116_444_736_000_000_000)
                        / 10_000_000,
                    identity: volume
                        .filter(|_| entry.FileId != 0)
                        .map(|v| (v, entry.FileId as u64)),
                    shared: false,
                    symlink: false,
                });
            }
            if next == 0 {
                break;
            }
            offset = end;
        }
        if !batch(entries, &handle) {
            return Ok(());
        }
    }
}

#[derive(Clone, Debug)]
pub struct Drive {
    pub root: PathBuf,
    pub label: String,
    pub filesystem: String,
    pub total: u64,
    pub free: u64,
    pub serial: u32,
}

pub fn volume_space(root: &Path) -> Option<(u64, u64)> {
    let w = wide(root);
    let (mut available, mut total, mut free) = (0, 0, 0);
    (unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut available, &mut total, &mut free) } != 0)
        .then_some((total, free))
}

pub fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Same representation at scan time and execution time; no 32-bit serial truncation.
pub fn handle_identity(file: &File) -> io::Result<(u64, u64)> {
    let mut info: FILE_ID_INFO = unsafe { zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as _,
            FileIdInfo,
            &mut info as *mut _ as _,
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if info.FileId.Identifier[8..].iter().any(|&b| b != 0) {
        return Err(io::Error::other("Unsupported non-NTFS file identity"));
    }
    Ok((
        info.VolumeSerialNumber,
        u64::from_le_bytes(info.FileId.Identifier[..8].try_into().unwrap()),
    ))
}

pub fn identity_handle(path: &Path, allow_delete: bool) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        // Attribute-only opens do not enforce sharing restrictions. Ancestor
        // guards request directory-list access too, so denying DELETE sharing
        // actually pins even an empty directory against removal or rename.
        .access_mode(FILE_READ_ATTRIBUTES | if allow_delete { 0 } else { FILE_LIST_DIRECTORY })
        .share_mode(
            FILE_SHARE_READ | FILE_SHARE_WRITE | if allow_delete { FILE_SHARE_DELETE } else { 0 },
        )
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(extended_path(path))
}
pub fn display_path(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}
pub fn is_volume_root(path: &Path) -> bool {
    let raw = display_path(path.to_path_buf())
        .to_string_lossy()
        .to_string();
    raw.len() == 3
        && raw.as_bytes()[0].is_ascii_alphabetic()
        && raw.as_bytes()[1] == b':'
        && raw.ends_with('\\')
}

fn local_drive_type(kind: u32) -> bool {
    matches!(kind, DRIVE_FIXED | DRIVE_REMOVABLE)
}

/// Avoid Windows "insert a disk" dialogs for empty removable devices.
struct MediaErrorMode(u32);
impl MediaErrorMode {
    fn new() -> Self {
        use windows_sys::Win32::System::Diagnostics::Debug::*;
        let old = unsafe { GetThreadErrorMode() };
        unsafe {
            SetThreadErrorMode(old | SEM_FAILCRITICALERRORS, ptr::null_mut());
        }
        Self(old)
    }
}
impl Drop for MediaErrorMode {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::SetThreadErrorMode(
                self.0,
                ptr::null_mut(),
            );
        }
    }
}

/// Ready local volumes, including external disks reported as fixed or removable.
pub fn drive(root: &Path) -> io::Result<Drive> {
    let _mode = MediaErrorMode::new();
    if !is_volume_root(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Scanning requires a whole local drive.",
        ));
    }
    let root = display_path(root.to_path_buf());
    let w = wide(&root);
    if !local_drive_type(unsafe { GetDriveTypeW(w.as_ptr()) }) {
        return Err(io::Error::other(
            "Choose a connected local disk or removable drive.",
        ));
    }
    let mut label = [0u16; 261];
    let mut fs = [0u16; 64];
    let mut serial = 0;
    if unsafe {
        GetVolumeInformationW(
            w.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            &mut serial,
            ptr::null_mut(),
            ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let (total, free) = volume_space(&root).ok_or_else(io::Error::last_os_error)?;
    let string = |s: &[u16]| {
        String::from_utf16_lossy(&s[..s.iter().position(|c| *c == 0).unwrap_or(s.len())])
    };
    Ok(Drive {
        root,
        label: string(&label),
        filesystem: string(&fs),
        total,
        free,
        serial,
    })
}

pub fn drives() -> Vec<Drive> {
    let mut drives = vec![];
    // Called by a background discovery worker. Never probe network or optical devices.
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = PathBuf::from(format!("{}:\\", char::from(b'A' + i)));
        if let Ok(drive) = drive(&root) {
            drives.push(drive);
        }
    }
    drives
}

#[derive(Clone, Debug, Default)]
pub struct Measurement {
    pub allocated: Option<u64>,
    pub identity: Option<(u32, u64)>,
    pub links: u32,
}

pub fn measure(path: &Path, attrs: u32) -> Measurement {
    let w = wide(&extended_path(path));
    measure_path(&w, attrs)
}

/// exFAT/FAT files have no resident data or hard links. Ask Windows for the
/// allocation directly, without NTFS-specific extent and identity queries.
pub(super) fn fat_allocation(path: &Path) -> Option<u64> {
    let w = wide(&extended_path(path));
    let handle = unsafe {
        CreateFileW(
            w.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let handle = DirectoryHandle(handle);
    let mut standard: FILE_STANDARD_INFO = unsafe { zeroed() };
    (unsafe {
        GetFileInformationByHandleEx(
            handle.0,
            FileStandardInfo,
            &mut standard as *mut _ as _,
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } != 0
        && standard.AllocationSize >= 0)
        .then_some(standard.AllocationSize as u64)
}

pub fn extended_path(path: &Path) -> PathBuf {
    // Win32 metadata APIs need the extended prefix even when Rust's filesystem APIs handle long paths.
    let raw = path.as_os_str();
    let text = path.to_string_lossy();
    if text.starts_with(r"\\?\") {
        path.to_path_buf()
    } else if let Some(unc) = text.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{unc}"))
    } else if path.is_absolute() {
        let mut prefixed = std::ffi::OsString::from(r"\\?\");
        prefixed.push(raw);
        PathBuf::from(prefixed)
    } else {
        path.to_path_buf()
    }
}

/// Query physical extents by file ID using the ordinary directory handle as a hint.
/// This avoids repeating pathname traversal and standard/identity metadata queries.
pub(super) fn allocation_from_entry(
    hint: &DirectoryHandle,
    directory: &Path,
    entry: &DirectoryEntry,
) -> Option<u64> {
    if entry.attributes
        & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE | FILE_ATTRIBUTE_REPARSE_POINT)
        != 0
        || entry.allocated.is_none()
    {
        return measure(&directory.join(&entry.name), entry.attributes).allocated;
    }
    if entry.allocated == Some(0) {
        return Some(0);
    }
    let mut descriptor: FILE_ID_DESCRIPTOR = unsafe { zeroed() };
    descriptor.dwSize = size_of::<FILE_ID_DESCRIPTOR>() as u32;
    descriptor.Type = FileIdType;
    descriptor.Anonymous.FileId = entry.identity.map(|(_, id)| id as i64).unwrap_or(0);
    let mut handle = if entry.identity.is_some() {
        unsafe {
            OpenFileById(
                hint.0,
                &descriptor,
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                FILE_FLAG_OPEN_REPARSE_POINT,
            )
        }
    } else {
        INVALID_HANDLE_VALUE
    };
    if handle == INVALID_HANDLE_VALUE {
        let w = wide(&extended_path(&directory.join(&entry.name)));
        handle = unsafe {
            CreateFileW(
                w.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
    }
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let handle = DirectoryHandle(handle);
    use windows_sys::Win32::System::{IO::DeviceIoControl, Ioctl::*};
    let start = STARTING_VCN_INPUT_BUFFER { StartingVcn: 0 };
    let mut extents: RETRIEVAL_POINTERS_BUFFER = unsafe { zeroed() };
    let mut returned = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle.0,
            FSCTL_GET_RETRIEVAL_POINTERS,
            &start as *const _ as _,
            size_of::<STARTING_VCN_INPUT_BUFFER>() as u32,
            &mut extents as *mut _ as _,
            size_of::<RETRIEVAL_POINTERS_BUFFER>() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    let error = if ok == 0 {
        unsafe { GetLastError() }
    } else {
        0
    };
    checked_extent_allocation(
        entry.allocated,
        ok != 0,
        error,
        returned,
        extents.ExtentCount,
        || {
            // An ID-based extent request can be denied while pathname metadata is
            // still readable. Obtain the authoritative FileStandardInfo allocation
            // rather than turning readable files into unknown/zero-sized entries.
            measure(&directory.join(&entry.name), entry.attributes).allocated
        },
    )
}

fn checked_extent_allocation(
    reported: Option<u64>,
    ok: bool,
    error: u32,
    returned: u32,
    extents: u32,
    fallback: impl FnOnce() -> Option<u64>,
) -> Option<u64> {
    if error == ERROR_HANDLE_EOF || (ok && returned >= 16 && extents == 0) {
        Some(0)
    } else if (ok || error == ERROR_MORE_DATA) && returned >= 16 && extents > 0 {
        reported
    } else {
        fallback()
    }
}

fn measure_path(w: &[u16], attrs: u32) -> Measurement {
    // Access 0 requests metadata, never file contents; OPEN_REPARSE_POINT prevents hydration/following links.
    let handle = unsafe {
        CreateFileW(
            w.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Measurement::default();
    }
    let mut standard: FILE_STANDARD_INFO = unsafe { zeroed() };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let mut allocated = if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileStandardInfo,
            &mut standard as *mut _ as _,
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } != 0
    {
        Some(standard.AllocationSize.max(0) as u64)
    } else {
        None
    };
    let got_info = unsafe { GetFileInformationByHandle(handle, &mut info) } != 0;
    // NTFS can report a resident stream's logical length as AllocationSize.
    // Starting VCN 0 returning EOF proves there are no separately allocated extents.
    // Residency is determined by extents, without a logical/allocation size cutoff.
    // Never query cloud/reparse entries.
    if allocated.is_some_and(|bytes| bytes > 0) && attrs & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        use windows_sys::Win32::System::{IO::DeviceIoControl, Ioctl::*};
        let start = STARTING_VCN_INPUT_BUFFER { StartingVcn: 0 };
        let mut extents: RETRIEVAL_POINTERS_BUFFER = unsafe { zeroed() };
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_GET_RETRIEVAL_POINTERS,
                &start as *const _ as _,
                size_of::<STARTING_VCN_INPUT_BUFFER>() as u32,
                &mut extents as *mut _ as _,
                size_of::<RETRIEVAL_POINTERS_BUFFER>() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if (ok == 0 && unsafe { GetLastError() } == ERROR_HANDLE_EOF)
            || (ok != 0 && returned >= 16 && extents.ExtentCount == 0)
        {
            allocated = Some(0);
        }
    }
    unsafe {
        CloseHandle(handle);
    }
    let mut measurement = Measurement {
        allocated,
        links: if got_info { info.nNumberOfLinks } else { 0 },
        identity: if got_info {
            Some((
                info.dwVolumeSerialNumber,
                ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
            ))
        } else {
            None
        },
    };
    if attrs & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE) != 0
        && measurement.allocated != Some(0)
    {
        let mut high = 0;
        unsafe {
            SetLastError(0);
        }
        let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
        if low != u32::MAX || unsafe { GetLastError() } == 0 {
            measurement.allocated = Some(((high as u64) << 32) | low as u64);
        } else {
            measurement.allocated = None;
        }
    }
    measurement
}

pub fn volume_file(root: &Path) -> io::Result<File> {
    let plain = display_path(root.to_path_buf());
    let raw = plain.to_string_lossy();
    let drive = raw.as_bytes();
    if drive.len() != 3 || !drive[0].is_ascii_alphabetic() || drive[1] != b':' || drive[2] != b'\\'
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose a drive root for NTFS index reading",
        ));
    }
    let device = PathBuf::from(format!(r"\\.\{}", &raw[..2]));
    std::fs::OpenOptions::new().read(true).open(device)
}

pub fn reveal(path: &Path) -> io::Result<()> {
    let mut command = std::process::Command::new("explorer.exe");
    if path.is_dir() {
        command.arg(path);
    } else {
        command.arg("/select,").arg(path);
    }
    command.spawn().map(|_| ())
}

pub fn ntfs_data(
    file: &File,
) -> io::Result<(
    windows_sys::Win32::System::Ioctl::NTFS_VOLUME_DATA_BUFFER,
    (u16, u16),
)> {
    use windows_sys::Win32::System::{IO::DeviceIoControl, Ioctl::*};
    #[repr(C)]
    struct Output {
        data: NTFS_VOLUME_DATA_BUFFER,
        extended: NTFS_EXTENDED_VOLUME_DATA,
    }
    let mut output: Output = unsafe { zeroed() };
    let mut returned = 0;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            FSCTL_GET_NTFS_VOLUME_DATA,
            ptr::null(),
            0,
            &mut output as *mut _ as _,
            size_of::<Output>() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        if returned < (size_of::<NTFS_VOLUME_DATA_BUFFER>() + 8) as u32
            || output.extended.ByteCount < 8
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing NTFS version information",
            ));
        }
        Ok((
            output.data,
            (output.extended.MajorVersion, output.extended.MinorVersion),
        ))
    }
}

/// Used only to bootstrap fragmented $MFT extents, never once per analyzed file.
pub fn ntfs_record(file: &File, reference: u64, record_size: usize) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::System::{IO::DeviceIoControl, Ioctl::*};
    let input = NTFS_FILE_RECORD_INPUT_BUFFER {
        FileReferenceNumber: (reference & 0x0000_ffff_ffff_ffff) as i64,
    };
    let mut output = vec![0u8; record_size + size_of::<NTFS_FILE_RECORD_OUTPUT_BUFFER>()];
    let mut returned = 0;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            FSCTL_GET_NTFS_FILE_RECORD,
            &input as *const _ as _,
            size_of_val(&input) as u32,
            output.as_mut_ptr() as _,
            output.len() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // The API can return the next lower in-use record. Require the requested slot.
    if returned < 12 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Truncated NTFS record query",
        ));
    }
    let actual = u64::from_le_bytes(output[..8].try_into().unwrap()) & 0x0000_ffff_ffff_ffff;
    let length = u32::from_le_bytes(output[8..12].try_into().unwrap()) as usize;
    if actual != (reference & 0x0000_ffff_ffff_ffff)
        || length != record_size
        || (returned as usize) < 12 + length
        || 12 + length > output.len()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Requested NTFS extension was not returned",
        ));
    }
    Ok(output[12..12 + length].to_vec())
}

#[cfg(test)]
mod allocation_tests {
    use super::*;
    use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_CDROM, DRIVE_RAMDISK, DRIVE_REMOTE,
    };

    #[test]
    fn external_fixed_and_removable_drives_are_supported_but_not_network_or_optical() {
        assert!(local_drive_type(DRIVE_FIXED));
        assert!(local_drive_type(DRIVE_REMOVABLE));
        for kind in [0, 1, DRIVE_REMOTE, DRIVE_CDROM, DRIVE_RAMDISK] {
            assert!(!local_drive_type(kind));
        }
        assert!(is_volume_root(Path::new("E:\\")));
        for path in ["E:", "E:\\folder", "1:\\", "\\\\server\\share\\"] {
            assert!(!is_volume_root(Path::new(path)));
        }
    }

    #[test]
    fn denied_extent_query_uses_path_metadata_instead_of_losing_allocation() {
        // Directory allocation can differ from FileStandardInfo. The fallback
        // must use the latter, including its unknown state, without a size cutoff.
        assert_eq!(
            checked_extent_allocation(Some(408), false, ERROR_ACCESS_DENIED, 0, 0, || Some(4096)),
            Some(4096)
        );
        assert_eq!(
            checked_extent_allocation(Some(408), false, ERROR_ACCESS_DENIED, 0, 0, || None),
            None
        );
        assert_eq!(
            checked_extent_allocation(Some(24), false, ERROR_HANDLE_EOF, 0, 0, || panic!(
                "A resident stream needs no fallback"
            )),
            Some(0)
        );
        assert_eq!(
            checked_extent_allocation(Some(8192), false, ERROR_MORE_DATA, 32, 1, || panic!(
                "A valid physical extent needs no fallback"
            )),
            Some(8192)
        );
    }
}
