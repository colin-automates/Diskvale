# Diskvale technical notes

[User guide](../README.md) · [Build instructions](build.md) · [Validation evidence](review-fixes-2026-10-10.md)

## How it works


1. **Read the NTFS index.** Diskvale opens the selected volume for reading and reads its Master File Table in 8 MiB chunks. It validates record boundaries, sector fixups, and file references, and resolves fragmented mappings and attribute-list extensions. It does not walk every file through Explorer or follow reparse targets.
2. **Build a tree.** Records become an indexed parent/child tree. Logical lengths, allocated data runs, modification times, and hard-link information are collected, then file counts and sizes are aggregated through folders.
3. **Lay out the map.** A squarified treemap divides the available area into nested rectangles. Small files can be grouped without discarding their bytes. Drawing and pattern geometry have budgets to keep very dense maps manageable.
4. **Prepare results in the background.** Scanning, filtering, file lists, and extension totals use background work. Cached results and virtualized tables reduce repeated work when switching views. The same search predicate drives the map and tables.
5. **Keep disk operations explicit.** Scanning reads metadata. Save/export writes only the chosen output file. Deletion is a separate confirmed Windows operation that revalidates the selected target before acting.

### Search syntax

Combine terms such as `ext:mp4 >1GB`, `archive <500MB`, `older:1y`, or `ext:zip newer:30d`. Size filters use the selected metric. GB/MB/KB are decimal; GiB/MiB/KiB are binary. Age units are days (`d`), weeks (`w`), 30-day months (`m`), and 365-day years (`y`). Files without a recorded date do not match age filters. Terms and extensions use Unicode lowercase matching in Map, Files and Types. Filters follow folder navigation.

### Understanding the sizes

**File size** retains the unnamed stream's logical length for ordinary files. **Size on disk** includes allocated storage from all `$DATA` streams, including named streams. Sparse holes do not count toward allocation; resident data has no separately allocated clusters. Named-stream logical lengths are recorded separately in snapshots and CSV. Multiple hard links share one allocation, assigned to one encountered path, so allocation attributed to an individual folder can depend on the link used.

Directory-owned `$DATA` streams, including on the volume root, appear once in a **Directory streams** bucket. Its logical measure sums those directory streams; its allocated measure counts their physical runs. The bucket participates in Map, Files, Types, search totals, and exports, carries no ordinary file count, and cannot be deleted. Its exported path identifies the owning directory, with `directory_stream_bucket=true`; it never invents a filesystem path. Directory aggregate rows include this bucket and descendants, so sum leaf CSV rows when reconciling the root total.

Directory index allocation and other volume overhead are excluded from file totals. A complete, allocation-known scan with directory-stream accounting and captured capacity/free-space data can show **Outside indexed files** for the remaining used space. A live scan is not an atomic filesystem snapshot, so changes during a scan can affect totals.

### Snapshots, background work, and deletion

Snapshot schema 2 adds NTFS identities and directory-stream accounting. Older snapshots remain readable when their tree and aggregate totals validate. Case-distinct sibling names are preserved when reopening and comparing captures; exact duplicate names remain invalid. Legacy captures retain an explicit warning that directory-stream data was not captured; saving them again does not manufacture that missing data. Comparisons require the same accounting profile. Saved snapshots and saved-MFT replays provide read-only browsing/export and never authorize deletion, regardless of serialized flags or notices.

Save, CSV export, and comparison preparation run in one bounded background job using the current immutable scan. The status bar shows busy/progress/error state and **Cancel job**. Replacement scans cancel the old job; an old comparison is discarded, and completed old outputs identify their originating scan. Native file dialogs stay on the UI thread. Outputs are staged beside the destination, flushed and synced, then replaced without first removing the old output. Cancellation after the replacement commit cannot undo it; this does not guarantee power-loss durability on every filesystem.

Deletion requires a fresh, complete live scan. Diskvale compares the full NTFS reference (including sequence) and volume identity through open handles, rejects reparse targets and paths through them, and holds ancestor handles that deny deletion/rename sharing throughout the Shell operation. The target handle allows Shell deletion, with another check immediately before operation and in the Shell callback. This is a best-effort identity guard: the Shell uses paths, and concurrent target renaming, alias changes, or changes after its callback can still race the operation. It is not atomic deletion of the scanned identity. A deleted or potentially partially changed result revokes all retained deletion targets and requires Rescan. Recycle refusal never automatically starts permanent deletion. Deleting a folder affects its current contents, including files added after scanning.

The sidebar pages direct children in groups of up to 256 while retaining selected ancestry. **More** reaches files and empty directories on later pages, and the last page cycles back; the Files view retains recursive largest-file ordering.

Ancestor identity guards request directory-list access as well as attributes: [attribute-only opens are exempt from sharing restrictions](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew). If this access cannot be obtained, deletion is refused. Target handles remain open through Shell completion, then close before verifying that the source path is gone; Windows can [defer removal until the last handle closes](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-deletefilew). Ancestor guards remain held through that final verification. These lifetime/access corrections preserve the best-effort limits above.

### Current scope

Diskvale is an early release with an **experimental NTFS reader**. The application scans whole NTFS drives; it does not scan arbitrary folders, network shares, or other filesystems. Opening a folder in the map explores the existing drive index. Unsupported or malformed metadata returns an error instead of silently falling back to another scanner. Ordinary records with nonresident attribute lists currently return an explicit unsupported-completeness error, even if a bulk read might recover their extension records. The supported MFT-bootstrap list reader is unchanged. Missing listed attributes, reused references, incomplete terminal stream coverage, and out-of-volume physical extents are rejected; cancelled scans are explicitly partial.

Large drives can require substantial memory. Full-volume correctness and performance parity with other disk tools has not been established. Live USN updates and duplicate-file discovery are not implemented.

## Source layout


| Path | Responsibility |
| --- | --- |
| `src/scanner/mft/` | Bulk NTFS reader, record validation, and tree construction |
| `src/model.rs` | Indexed tree, paths, totals, and scan metadata |
| `src/treemap.rs` | Squarified nested map layout and grouped tiles |
| `src/query.rs`, `src/file_types.rs` | Search predicates and color categories |
| `src/ui.rs`, `src/ui/` | Native egui interface, navigation, tables, and background result caches |
| `src/recycle.rs` | Confirmed, revalidated Windows recycling/permanent deletion |
| `src/main.rs`, `build.rs` | Startup, administrator checks, icon, and Windows resources |
| `tests/`, `examples/` | Regression tests, saved-scan UI preview, and synthetic demo capture |
| `assets/brand/`, `tools/demo-video/` | Editable branding and reproducible demo production |
| `scripts/` | Release packaging, resource checks, and license notices |

The legacy filesystem walker in `src/scanner/` is retained for library regression tests; desktop and CLI scan routes use the bulk NTFS reader exclusively. Diskvale is an independent project and does not include WizTree code or binaries.
