# Diskvale 0.3.26 — unpublished parent-reference test candidate

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

Executed automated checks passed: formatting, locked tests (**139 library + 7 integration tests passed; 3 opt-in tests ignored**) and strict all-target Clippy. The 44 MFT regressions include the same synthetic parent error before/after numeric diagnostics and complete validated retry behavior. The actual C: failure's cause and recovery remain unconfirmed until the user tests this rebuilt candidate.

## Build and manual C: test

After formatting, locked tests and strict Clippy pass, commit the focused source and build/package from the clean commit:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build_release.ps1
powershell -ExecutionPolicy Bypass -File scripts/package_release.ps1 -TestCandidate -OutputDirectory artifacts/candidates/v0.3.26
python scripts/check_brand_resources.py
```

The earlier 0.3.25 candidate files are retained. `BUILD_INFO.json` records this candidate's source SHA/toolchain/EXE checksum; `SHA256SUMS.txt` covers its exact assets. Public v0.3.24 assets, tags and download buttons remain unchanged.

Open the new **0.3.26** candidate, approve the normal administrator prompt, choose **C:** and click Scan. Wait for the entire scan to complete. If it still fails, share the complete error banner, including numeric record/sequence details. Do not send a full snapshot or private filenames.

If the scan completes, verify Map navigation, Files/Types, search, saving/reopening a snapshot and CSV export. Reopened results must remain read only. No deletion test is needed.

The session cannot independently verify the elevated GUI scan. The real C: result is still pending, and no public release may be published until the same drive passes. A bounded reread cannot guarantee consistency on a changing volume or resolve genuinely invalid/unsupported metadata; experimental reader and non-atomic scan limits remain.
