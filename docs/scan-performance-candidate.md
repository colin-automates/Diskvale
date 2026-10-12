# Diskvale 0.3.27 — scan performance candidate

This candidate reduces allocation and memory-copy work in the existing read-only NTFS scanner and records separate stage timings. The public download remains 0.3.26 while the candidate is evaluated. No dependency, scanner architecture, deletion-authority or public-download change is included.

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

The existing generated 100,000-file flat-MFT fixture is used for paired release-mode comparisons against the preserved 0.3.26 benchmark. Both builds must produce 100,003 nodes, 921,616,384 allocated bytes and zero per-file metadata queries. Runs alternate in ABBA order, using the same local compiler, target, flags and fixture; report medians rather than selecting a fastest run. This fixture tests bulk parsing/tree work, not full-volume I/O or external-list compatibility. Live C: timing summaries remain private local artifacts and contain no per-file capture.

Build from the clean tested commit:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build_release.ps1
powershell -ExecutionPolicy Bypass -File scripts/package_release.ps1 -TestCandidate -OutputDirectory artifacts/candidates/v0.3.27-final
python scripts/check_brand_resources.py
```

Open the candidate, approve normal administrator access, choose C: and click Scan. Verify the complete scan and Map, Files, Types, search and save/reopen/export behavior. Compare Scan notes and elapsed times under similar activity; changing live metadata, cache state and retries can affect results. Keep snapshots and filenames private. A measured reduction on one fixture or drive does not establish speed/correctness parity with other tools.

Stage results determine the next optimization. Pipelining reads and parsing, or adding parser workers, should target a measured bottleneck and retain bounded memory, independent external-read positioning, deterministic assembly, cancellation and the same validation checks.
