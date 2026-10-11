# Diskvale review fixes — 2026-10-10

All eight findings were confirmed against the clean starting checkout at `5ee6168b734b0266b89659e641191830058e0ff5` and addressed locally. The native Rust/egui layout, bulk MFT scan, bounded result caches, recursive largest-files view, and separate deletion confirmations remain in place. Implementation and validation were completed locally before the subsequent request to commit and push. No version bump, publication, GitHub release, or live-drive scan was performed.

Validation host: Windows x64, version `10.0.26200`; Rust `1.98.1 (48a229cea 2026-09-01)`, `x86_64-pc-windows-msvc`; AMD Ryzen 7 9800X3D, 8 cores / 16 threads, 31.14 GiB RAM. Repository instructions were read before editing; no applicable `AGENTS.md` was found. The starting worktree was clean.

## Findings and regression evidence

| Finding | Disposition and changed files | Regression evidence after repair |
| --- | --- | --- |
| 1. Deletion identity and revalidation | Implemented with explicit best-effort race limits. `src/model.rs`, `src/scanner/mft/tree.rs`, `src/scanner/mft.rs`, `src/scanner/windows.rs`, `src/recycle.rs`: preserve the full reference and sequence; sample the FILE_ID_INFO volume serial once; verify identity through live handles; hold ancestor guards through Shell; recheck before operation and in the callback. | `identity_rejects_same_size_replacement_directory_and_volume_and_keeps_contents_changes`: unchanged targets accepted, same-size file and directory replacements and volume mismatch refused, current directory contents intentionally allowed. `deterministic_pre_operation_replacement_is_refused_and_ancestors_are_pinned`: replacement at the hook never reaches the injected backend; ancestor rename is denied while guarded. Callback and UI tests preserve recycle veto and separate permanent confirmation. |
| 2. Runtime provenance | Implemented. `src/model.rs`, `src/scanner/mft.rs`, `src/scanner/mft/tree.rs`, `src/recycle.rs`, `src/ui.rs`, `src/ui/layout.rs`: private runtime capability with live/snapshot/replay/incomplete states and shared revocation, skipped by serde. Target creation and execution both gate authority. Snapshot import validates structure and checked aggregate totals, then recomputes summary counts. | `serialized_flags_and_direct_helpers_cannot_mint_or_reuse_live_authority`, `helper_revokes_all_retained_targets_after_operation_but_preserves_recycle_veto`; production replay/cancellation tests explicitly assert ineligibility. Import tests reject non-directory roots, children on files, cycles, unsafe components, duplicate names, overflows, and forged totals. Legacy missing accounting stays missing. |
| 3. Scan-bound UI state | Implemented. `src/ui.rs`, `src/ui/layout.rs`: centralized installation clears IDs, map geometry, menus/popups, tree pages, selection, filter receivers, and result caches before keyboard input. Requests, jobs, and confirmations carry generations. Starting a new result request also revokes old deletion authority. | `accepting_replacement_scan_before_arrow_input_clears_all_old_actions`: real egui frames accept smaller and same-length replacement trees with an ArrowRight event and an open old popup; no panic or unrelated selection. `late_old_scan_request_and_retained_confirmation_are_harmless`: old results and retained actions are discarded. Existing keyboard/navigation regressions pass. |
| 4. Directory-owned streams | Implemented. `src/model.rs`, `src/query.rs`, `src/scanner/mft/format.rs`, `src/scanner/mft/tree.rs`, `src/ui.rs`, `src/output.rs`: own bytes plus one synthetic bucket per owner; aggregate once. Bucket paths resolve to owners and cannot authorize deletion. Named logical bytes remain separate from ordinary file length. | `regression_directory_and_root_streams_are_accounted`: resident and nonresident streams on root and child directories, including descendants; root allocation rises by exactly 8,192 bytes; Types, query totals, treemap buckets, and CSV leaf allocation reconcile. `virtual_coverage_accepts_preallocation_and_sparse_directory_streams` checks sparse directory ADS and file ADS semantics. Existing bulk hard-link and compressed/sparse allocation fixtures pass. |
| 5. Stream completeness | Implemented validation; ordinary nonresident attribute lists are explicitly unsupported. `src/scanner/mft/format.rs`, `src/scanner/mft.rs`, `src/scanner/mft/source.rs`: retain full listed identities, verify matching attributes and record sequences, contiguous segments and terminal virtual coverage, checked arithmetic and live-volume physical bounds. No walker fallback or fabricated complete totals. | `regression_missing_terminal_extent_is_rejected_by_bulk_parser`; `missing_terminal_extension_and_preallocated_tail_are_not_complete`; `listed_attribute_identity_and_nonresident_list_completeness_are_enforced`; `bulk_data_extents_are_bounded_by_known_volume_geometry`. Missing/wrong attributes and nonresident lists error explicitly. Valid resident, fragmented, sparse, compressed, and preallocated fixtures pass through the production bulk parser; reused references are rejected. |
| 6. Background output/comparison | Implemented. `src/ui.rs`, `src/ui/layout.rs`, `src/output.rs`: one active output/comparison worker, immutable `Arc<Scan>` sharing, busy/progress/error display, cancellation, generation-bound completion. Dialogs stay on the UI thread; baseline parsing, validation, and path-map construction move to the worker. | `blocked_io_worker_leaves_frames_and_navigation_responsive_and_discards_old_comparison`: 20 egui CPU frames in **3.3147 ms**, Files-tab input processed while an injected worker waits, second worker refused, old comparison discarded. `io_cancel_error_disconnect_and_old_export_are_reported`; `large_generated_snapshot_job_shares_scan_and_validates_off_thread`: 20,000-file round trip, cancellation, failures, disconnects, and truthful old-scan output labels. |
| 7. Failure-safe output | Implemented. New `src/output.rs`; `src/ui.rs` UI and CLI callers: unique `create_new` staging beside the destination, explicit serialization/flush/sync completion, close before same-volume replacement, no pre-removal of old output. CLI errors include output context and exit 1. | `output_survives_write_flush_cancel_and_replacement_failure`: every injected failure preserves previous bytes and cleans the temporary file. `output_creates_and_overwrites_unicode_destination`: new file and successful overwrite. `windows_locked_destination_preserves_old_output_and_cleans_stage`: real Windows replacement failure against a locked disposable destination preserves old bytes. |
| 8. Sidebar overflow | Implemented. `src/ui.rs`, `src/ui/layout.rs`: virtualized pages of up to 256 direct children plus selected ancestry. More cycles through pages including directories; recursive Files semantics stay intact. | `every_empty_overflow_directory_is_reachable_in_bounded_pages`: both 300 and 620 empty-directory fixtures, all pages and navigation into every directory, actual More next-page values, final-page wraparound, bounded row count, selected item beyond the first page. Existing mixed-entry, selected-descendant, sort, and zero-visibility regressions pass. |

Focused regressions were added before repair where practical. Two were executed against the original code and failed: the bulk parser accepted a logical 8 KiB stream with only one mapped 4 KiB cluster, and directory/root stream allocation was missing. Both pass after repair. The other original defects were established by inspection of the baseline; their new tests were executed after implementation, so this report does not claim observed pre-fix runtime failures for those cases. Initial restricted-process fixture errors were access-denied failures, not scanner correctness evidence; the ordinary suite was rerun with normal Windows filesystem access.

All normal test mutations use newly created disposable fixtures. Normal tests do not call successful Shell deletion. The ignored Shell tests now require `DISKVALE_DELETE_FIXTURE_TESTS=1`, exact-name invocation, and workspace fixture confinement; neither was executed in this session.

## Executed validation

Commands ran from the repository root. Cargo was made available with:

```powershell
$env:PATH = 'C:\Users\Colin\.cargo\bin;' + $env:PATH
```

| Exact command | Outcome |
| --- | --- |
| `cargo fmt --all` | Applied formatting. |
| `cargo fmt --all -- --check` | Passed. |
| `cargo test --locked` | Passed: **116 library tests + 7 integration tests = 123**, zero failures. Three opt-in tests ignored by default. Binary/doc tests contain zero cases. |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed, no warnings. |
| `cargo test --locked --lib ui::layout::tests::blocked_io_worker_leaves_frames_and_navigation_responsive_and_discards_old_comparison -- --exact --nocapture` | Passed; measured 20 blocked-worker CPU frames in 3.3147 ms. |
| `./scripts/build_release.ps1` | Passed; final x64 portable build completed in 47.31 seconds. |
| `./scripts/package_release.ps1 -OutputDirectory artifacts/review-validation/package` | Passed; EXE, portable ZIP, notices, existing demo, and SHA-256 sums created locally. Isolated output directory preserves previous local packages. |
| `python scripts/check_brand_resources.py --output artifacts/review-validation/resource-checks` | Passed; 10 embedded icon sizes match source, Windows icon extraction and version metadata passed, no separate Visual C++ runtime imports. Application execution and raw-volume reads were not part of this check. |
| `git diff --check` | Passed. |

Final local executable SHA-256: `3ffc2697036b7b8e8cc7c7420c84159470bb029c92e61b1eba3c107cd566992b`; portable ZIP SHA-256: `4da65dcd59f1b78a12e19224fa8438d915a252d0f64dc1d3087350a3db25d7bb`. Outputs are under `artifacts/review-validation/package` and remain ignored build artifacts, not a published release.

The read-only saved-scan timing test was explicitly named and run for the isolated baseline and implementation, rather than enabling every ignored test:

```powershell
$env:CANOPY_TIMING_SNAPSHOT = (Resolve-Path artifacts/review-validation/generated-100k.json).Path
$env:CANOPY_TIMING_OUTPUT = Join-Path (Get-Location) artifacts/review-validation/baseline-view-timing.json
cargo test --locked --manifest-path artifacts/review-validation/baseline/Cargo.toml --target-dir target --bin canopy ui::layout::tests::measure_saved_scan_view_switches -- --exact --ignored --nocapture
$env:CANOPY_TIMING_OUTPUT = Join-Path (Get-Location) artifacts/review-validation/updated-view-timing.json
cargo test --locked --lib ui::layout::tests::measure_saved_scan_view_switches -- --exact --ignored --nocapture
```

The benchmark example accepts a supplied generated/sanitized flat MFT and only reads it. For repeat measurement, build that example against each checkout and run its resulting executable with `artifacts/review-validation/generated-100k.mft`. The baseline copy under `artifacts/review-validation/baseline` was extracted from `git archive` of the starting commit; only the same read-only benchmark example was added there. Preparing this baseline copy did not alter repository history or create a release.

```powershell
# Reproduce the parser measurements with the existing generated input:
cargo build --locked --manifest-path artifacts/review-validation/baseline/Cargo.toml --target-dir target --example scan_fixture_benchmark
./target/debug/examples/scan_fixture_benchmark.exe artifacts/review-validation/generated-100k.mft
cargo build --locked --example scan_fixture_benchmark
./target/debug/examples/scan_fixture_benchmark.exe artifacts/review-validation/generated-100k.mft
```

The recorded peak-working-set runs launched the same benchmark/test executables with `Start-Process -WindowStyle Hidden`, redirected stdout/stderr into the ignored artifact directory, and sampled `PeakWorkingSet64` while the process was alive. Both parser processes and both named view-test processes exited 0.

## Timing and memory comparison

Generated inputs contain fictional `Z:\` paths and no user files. Flat MFT: 100,016 slots × 1,024 bytes = **102,416,384 bytes**, 4 KiB clusters, 100,000 ordinary files plus metadata/root = 100,003 nodes; baseline and updated indexed allocation both **921,616,384 bytes**, and both report **zero per-file metadata queries**. View snapshot: 100,000 files and 101 directories = **100,101 nodes**, **20,642,545 bytes** JSON.

The parser example ran five times per process using the repository's optimized development/test profiles, not the portable release build. Peak working set was sampled from the launched process at approximately 5 ms intervals; all process runs exited 0. The view test renders production egui CPU frames at 1,400 × 900 with no GPU presentation. Single-machine, cached-file measurements include process/startup and sampling noise; they are not cold-disk or full-volume benchmarks.

| Measure | Baseline | Updated |
| --- | ---: | ---: |
| Five-run parser median, ms | 83.6434 | 91.6825 |
| Parser read/parse median, ms | 60.8721 | 62.5975 |
| Tree-build median, ms | 21.5532 | 27.8420 |
| Parser process peak working set, bytes | 56,516,608 | 65,904,640 |
| `size_of::<Node>()`, bytes | 104 | 144 |
| Initial Map first CPU frame, ms | 11.4942 | 12.2708 |
| Initial Map data ready, ms | 32.7517 | 33.5554 |
| Files first CPU frame, ms | 0.6679 | 0.6694 |
| Types first CPU frame, ms | 0.5119 | 0.4815 |
| Map return first CPU frame, ms | 0.1669 | 0.1585 |
| View process peak working set, bytes | 40,947,712 | 44,941,312 |

The parser median increased **9.6%**, and sampled parser peak memory increased **16.6%**. Additional identity/accounting fields contribute to this cost; the measurements do not isolate every cause. Repeated view frames remained below 0.5 ms in these runs, but the initial Map frame was somewhat slower. No memory reduction, broad performance improvement, or WizTree parity is claimed. Raw generated timing JSON and process summaries remain in the ignored `artifacts/review-validation` directory.

## Compatibility and remaining guarantees

Snapshot schema **2** adds optional full references, reparse/synthetic markers, directory-owned logical/allocated bytes, named-stream logical bytes, volume identity, and a directory-stream capture profile. Serde defaults keep compatible legacy snapshots readable after graph/component/aggregate validation; a newer unsupported schema, unsafe tree, or contradictory accounting is rejected. Runtime authority is never serialized. Legacy capture warnings remain, resaving does not fabricate missing directory streams, mismatched accounting profiles cannot be compared, and incomplete legacy accounting suppresses an outside-indexed remainder. File logical-size semantics remain the unnamed stream length, independently suitable for live length revalidation.

Ordinary nonresident attribute lists are refused explicitly rather than claiming completeness. Existing supported MFT-bootstrap nonresident-list handling remains tested. Saved flat MFT captures do not encode the live volume's physical cluster bound; replay still validates arithmetic, attribute identities, and virtual coverage but cannot reconstruct that missing volume geometry. Replay remains read-only. Cancellation checks are cooperative; validation/aggregation and OS flush/replacement calls are not instantly interruptible. Output cancellation before commit preserves the destination; cancellation after commit cannot undo replacement. Temporary cleanup is best effort if the OS itself refuses cleanup. File sync plus rename does not establish universal directory-entry crash/power-loss durability.

Windows identity verification uses [FILE_ID_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info) and documented [CreateFile sharing/open flags](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew). Holding ancestors without DELETE sharing blocks ordinary concurrent ancestor rename/removal; reparse objects are rejected. Holding the target with DELETE sharing allows the Shell operation and pins that object against identity reuse, but does **not** bind Shell pathname resolution atomically to that handle. The extra pre-operation and [PreDeleteItem callback](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperationprogresssink-predeleteitem) checks narrow the window. Target renaming/replacement or alias/reparse changes after the last check can still race; a directory's current descendants can also change. Deletion therefore remains **best effort**, with unsafe/unsupported targets refused and fresh identity checks required at each separately confirmed action. The default remains recycling; a callback veto never automatically retries permanently.

Output replacement uses [MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw) with REPLACE_EXISTING and WRITE_THROUGH on a same-directory staging file, without COPY_ALLOWED or removing the destination first. Injected failures and the Windows locked-destination test establish failure preservation for the exercised cases, not resilience to every filesystem failure or power interruption.

Not performed: real Recycle Bin/permanent-deletion integration (separate opt-in absent), full live-volume scanning, native GPU/window visual acceptance, cold-disk performance, or testing on additional Windows versions/hardware. Synthetic egui frame tests and Windows handle/rename/output fixture tests were performed; those do not substitute for the unexecuted checks.

## Complete changed-file list

- `src/model.rs`, `src/query.rs`, `src/recycle.rs`, `src/scanner/windows.rs`.
- `src/scanner/mft.rs`, `src/scanner/mft/format.rs`, `src/scanner/mft/source.rs`, `src/scanner/mft/tree.rs`, `src/scanner/mft/tests.rs`.
- `src/ui.rs`, `src/ui/layout.rs`, new `src/output.rs`.
- `src/lib.rs`, `src/main.rs`: share the UI through the library so synthetic UI tests can construct runtime fixture authority without exposing a public capability constructor.
- `examples/map_review.rs`, `examples/demo_video.rs`: adapt example/test compilation to the shared UI; new `examples/scan_fixture_benchmark.rs` provides read-only fixture measurements.
- `README.md`, `CONTRIBUTING.md`, this report: accounting, snapshot compatibility, job/output behavior, deletion guarantees, opt-in instructions, and validation evidence.

No dependency, Cargo version, release workflow, brand asset, or production deployment changes were made.

## Review follow-up fixes

Two confirmed findings from the subsequent review were repaired after commit `a0cc5bd`:

- Snapshot validation checks exact duplicate sibling names, preserving case-distinct names such as `Foo.txt` and `foo.txt`. A production save/load fixture verifies both names, totals, separate comparison keys, and read-only provenance; exact duplicate names are still rejected. No snapshot schema change is needed.
- Query parsing uses Unicode lowercase, matching filename normalization. Regressions cover accented mixed-case terms and extensions, combined size filters, and expanding lowercase characters. Production egui frames verify matching files, Map bytes/counts/tiles, and Types totals across view switches with both size metrics, including a query with no matches.

`cargo test --locked --lib review_regression_` first failed all three new tests against the previous implementation, then passed all three after the repairs. `cargo fmt --all -- --check`, `cargo test --locked` (**126 passed, 3 ignored**), `cargo clippy --locked --all-targets -- -D warnings`, and `git diff --check` passed. The suite used synthetic data and disposable JSON fixtures; no case-sensitivity setting on a real directory was changed and no live drive or successful Shell deletion was exercised.

Follow-up Windows validation passed: `./scripts/build_release.ps1` (44.26 seconds), `./scripts/package_release.ps1 -OutputDirectory artifacts/review-validation/followup-fixes/package`, and `python scripts/check_brand_resources.py --output artifacts/review-validation/followup-fixes/resource-checks`. The 10 embedded icon sizes match source, version metadata and Windows extraction pass, and no separate Visual C++ runtime is required. The follow-up executable SHA-256 is `9ede17dad2ff9f57176d413aa0a49a1e0f4ebe10e9d70704470328c74d656c6e`; ZIP SHA-256 is `0af43e8d3122a52d0aa1572e04ec67f0730996ad0e9cc782779fce1ec2b8c134`. The earlier package and timing evidence above describe the initial eight-finding repair and are retained separately. Native window/GPU acceptance was not performed; the regression checks exercised production egui CPU frames.
