# Diskvale 0.3.26 — parent-reference candidate validation

The first 0.3.25 live test on C: got past attribute-list parsing but failed during tree construction:

```text
NTFS directory parent is missing or reused; rescan the live volume
```

That message does not identify the affected record and does not establish whether the cause is concurrent namespace changes or consistently invalid/unsupported metadata. The same error was reproduced through the synthetic volume/bootstrap/bulk/tree pipeline by making a child's parent reference expect sequence 1 while the scanned parent had sequence 2. The diagnostic regression failed before the change because the record identities were absent from the error.

## Candidate behavior

- A typed parent-reference failure permits **one** fresh live index read, using the same read-only volume handle and fresh geometry/bootstrap metadata. All attributes, extents, record sequences, extension ownership and parent references must pass again.
- A successful reread reports the retry in Scan notes. Allocation and file counts come from the successful attempt; discarded attempts do not double-count records.
- A persistent mismatch remains an error. Its message identifies the child record/sequence, expected parent record/sequence and observed parent sequence/type when available. No filenames or paths are included.
- Missing nodes are not silently omitted or attached under a made-up parent. No sequence mask, walker fallback, repair/defragmentation command or deletion change is included.
- Cancellation prevents a retry and returns an explicitly partial, nondeletable result. Saved-MFT replay remains read only and does not access/retry a live drive. Unrelated malformed metadata is not retried.

Regressions exercise recovery after a changed first read, failure after both inconsistent reads, a parent that is not a directory, progress reset, cancellation, unchanged replay restrictions, and success/fatal-error paths that read the index only once. Existing attribute-list and accounting regressions remain enabled.

Executed automated checks passed: formatting, locked tests (**139 library + 7 integration tests passed; 3 opt-in tests ignored**) and strict all-target Clippy. The 44 MFT regressions include the same synthetic parent error before/after numeric diagnostics and complete validated retry behavior. Windows CI also passed for the tested source at `054372fb9514113a0846154c19e37e2dfbb7ee96`. The original C: failure's exact cause was not established; successful live acceptance is recorded below.

## Build and manual C: test

After formatting, locked tests and strict Clippy pass, commit the focused source and build/package from the clean commit:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build_release.ps1
powershell -ExecutionPolicy Bypass -File scripts/package_release.ps1 -TestCandidate -OutputDirectory artifacts/candidates/v0.3.26
python scripts/check_brand_resources.py
```

The earlier 0.3.25 candidate files are retained. `BUILD_INFO.json` records each build's source SHA/toolchain/EXE checksum; `SHA256SUMS.txt` covers its exact assets. The existing v0.3.24 tag and binaries remain unchanged. The public 0.3.26 package is rebuilt from the clean release commit, with normal release instructions rather than the unpublished candidate label.

Open the new **0.3.26** candidate, approve the normal administrator prompt, choose **C:** and click Scan. Wait for the entire scan to complete. If it still fails, share the complete error banner, including numeric record/sequence details. Do not send a full snapshot or private filenames.

If the scan completes, verify Map navigation, Files/Types, search, saving/reopening a snapshot and CSV export. Reopened results must remain read only. No deletion test is needed.

The session cannot independently inspect the elevated GUI scan. On 2026-10-12, the user confirmed that the 0.3.26 candidate completed a **full scan of the same C: drive**. This is user-confirmed live acceptance of the scanner repair, rather than an independently observed scan or a full-volume correctness comparison. The tested candidate was built from `054372fb9514113a0846154c19e37e2dfbb7ee96`; its EXE SHA-256 is `1b201b90c6811717d4506455ac3273e4a293f047a9422a57a0937142ba14eb72`.

The user also confirmed that Map, Files, Types, search, snapshot save/reopen and CSV export all worked. No private drive capture or filenames were uploaded, and no destructive real-drive test was performed. Saved-result deletion restrictions are covered by automated regressions. These confirmations clear the same-drive release gate for the 0.3.26 early preview.

A bounded reread cannot guarantee consistency on a changing volume or resolve genuinely invalid/unsupported metadata; experimental reader and non-atomic scan limits remain. This acceptance does not establish correctness or performance parity across other drives or every supported NTFS layout.
