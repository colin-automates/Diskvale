# Diskvale 0.3.27 — scan performance validation

Diskvale 0.3.27 reduces allocation and memory-copy work in the existing read-only NTFS scanner and records separate stage timings. The user confirmed that the extracted candidate worked before authorizing publication. The [0.3.27 preview](https://github.com/colin-automates/Diskvale/releases/tag/v0.3.27) retains the existing dependencies, scanner architecture and deletion safeguards.

## Changes

- Sector fixups copy the update-sequence array into bounded stack storage; the common 1 KiB record uses six bytes. Larger supported geometries remain covered, and the helper preserves its older behavior outside live geometry bounds.
- Attribute bounds, forms, names and the terminator are validated before parsing. The bulk parser then uses an immutable borrowed attribute view, avoiding a temporary attribute-vector allocation per record. Bootstrap callers retain their existing collected-view API.
- Filename decoding avoids the intermediate UTF-16 allocation. Validated ASCII uses one allocation for the final string; other names use strict UTF-16 decoding, including supplementary characters and rejection of unpaired surrogates.
- A DOS alias/long-name pair with the same parent is normalized without creating a hash set. Other name and hard-link combinations retain full sorting, deduplication and parent handling. Ordinary name vectors reserve two entries instead of the default four; records without names allocate none.
- Progress file/folder counts are published once per bulk chunk. Cancellation is still checked per record. Tree capacity is reserved from checked normalized-name counts, with at most one stream bucket per directory, and directory names move into nodes instead of being copied.
- External-list reads reuse a bounded scratch buffer and the bulk reader's known physical cursor, avoiding an extra cursor-query syscall per list. Every list is still read again from the volume; metadata bytes are not cached. Success, error and cancellation paths restore the sequential cursor, and logical EOF still excludes sector padding.

All attribute-list identities, extent/volume bounds, bootstrap/bulk checks, stream accounting, parent sequences, hard links and explicit error paths remain enabled. The existing one-retry limit remains unchanged. Replayed, saved, cancelled and stale results retain their deletion restrictions.

## Stage timings

Scan notes and CLI summaries now expose captured times for bootstrap, bulk index reads, parsing/validation, external-list resolution, extension merging and tree construction. These stages accumulate across failed and successful live attempts, so a retry is visible rather than disappearing from the timing report. External-list resolution is excluded from parsing time. Setup/cleanup and other wrapper overhead can make total scan time greater than the sum of recorded stages.

Timings are taken per chunk or external list, not per ordinary record. Existing combined `mft_read_seconds` and `tree_build_seconds` fields retain their meanings. The optional `mft_timings` snapshot field has a default for older snapshots and cannot grant live deletion authority; the snapshot schema remains version 2. Saved snapshots display the captured scan's timing, rather than claiming that it measures reopening.

## Validation

The standard suite passed with **144 library + 7 integration tests; 3 opt-in tests ignored**. All **49 NTFS regressions** passed, including new coverage for update-sequence overlap/buffer limits, supplementary Unicode and malformed names, prevalidation before external reads, DOS alias versus separate hard-link names, accumulated retry timings, fresh external-list bytes/cursor restoration and older snapshot compatibility. Formatting and strict all-target Clippy passed. Shell-deletion integrations were not enabled for this performance change.

Paired comparisons used the same local Rust 1.98.1 MSVC toolchain, static CRT and release flags, alternating builds in ABBA order. The generated 100,000-file flat-MFT fixture took a median **62.0893 ms on 0.3.26 versus 48.3009 ms on 0.3.27** across 20 runs per build, a **22.21% reduction**. Every run produced 100,003 nodes, 921,616,384 allocated bytes and zero per-file metadata queries. This fixture tests bulk parsing/tree work, not full-volume I/O or external-list compatibility.

Four read-only C: scans per build took a median **11.3844 s versus 9.4880 s**, a **16.66% reduction**. Median peak working set fell from **2599.33 MiB to 2135.91 MiB**, a **17.83% reduction**. All eight scans completed without cancellation, read errors, unknown allocations or parent-reference retries. Live namespace activity prevents an atomic totals comparison. These results describe this drive and local toolchain, not guaranteed gains on every system or measurements of the CI-built download. Raw timing summaries remain private ignored artifacts; no per-file capture, snapshot or CSV was uploaded.

The accepted candidate was built from `f185c9725fb61224d03e17e449cf7bd93a8fc3b5`, with EXE SHA-256 `d55323ccc59bdba05c6fcc4bce2edb65be665ac30f1f0ce514f9d0354de8a160`. The public package uses the clean release commit containing the same application code and final release documentation; its `BUILD_INFO.json` records its source, toolchain and checksum. Candidate GUI acceptance is user-confirmed.

For a separate unpublished test package, build from a clean tested commit:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build_release.ps1
powershell -ExecutionPolicy Bypass -File scripts/package_release.ps1 -TestCandidate -OutputDirectory artifacts/candidates/v0.3.27-final
python scripts/check_brand_resources.py
```

Public packaging omits `-TestCandidate`; follow [release maintenance](release.md) for clean-source build, CI, provenance and public-download checks. For subsequent candidates, approve normal administrator access, choose the same NTFS drive and click Scan. Verify complete scanning, navigation/search and save/reopen/export behavior. Compare Scan notes and elapsed times under similar activity; changing live metadata, cache state and retries can affect results. Keep snapshots and filenames private. A measured reduction on one fixture or drive does not establish speed/correctness parity with other tools.

Stage results determine the next optimization. Pipelining reads and parsing, or adding parser workers, should target a measured bottleneck and retain bounded memory, independent external-read positioning, deterministic assembly, cancellation and the same validation checks.
