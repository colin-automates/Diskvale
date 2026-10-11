# Diskvale 0.3.25 — unpublished NTFS test candidate

This repair addresses the live-scan failure reported in v0.3.24 at `58beefd3a7387c9f3f1d6be04e66e848c51ed0ae`. Public downloads remain on v0.3.24. Do not tag or publish 0.3.25, or change the download buttons, until the same failing drive completes the live smoke test below.

## Diagnosis and regression

Bootstrap could read an external `$ATTRIBUTE_LIST` to locate the fragmented `$MFT`, but discarded that validation context. Bulk parsing then rejected every nonresident list, including record zero.

Before changing the reader, `volume_backed_nonresident_record_zero_list_completes_full_pipeline` failed with:

```text
NTFS record 0: Unsupported nonresident attribute list in bulk NTFS record; completeness cannot be verified
```

The fixture contains actual list bytes in volume clusters, a fragmented MFT mapping, its listed extension, a root and ordinary files. It exercises production bootstrap, mapped bulk reading, attribute validation, extension merging and tree construction. After the fix it scans all 64 records, preserving 10 files, 3 folders, 1 shared link, 1,141,328 logical bytes and 94,208 allocated bytes.

The same resolver now handles ordinary file/directory lists. Regressions cover fragmented list storage with a mid-sector logical tail, wrong/missing identities and owners, bootstrap/bulk changes, inconsistent/truncated/out-of-volume mappings, cancellation, one merge per extension, and continuation into the next 8 MiB bulk chunk. Generated snapshot/CSV tests and existing provenance tests preserve saved-result deletion restrictions.

The resolver preserves the sequential cursor on success and failure. Bootstrap checks retain all required entry identities, including non-data attributes; matching only the mapping used to locate `$MFT` is insufficient. Missing external data in flat saved-MFT files still produces an explicit error and never triggers a live-drive read. No scanner fallback or dependency changes are included.

Automated candidate validation passed: `cargo fmt --all -- --check`, `cargo test --locked` (**133 library + 7 integration tests passed; 3 opt-in tests ignored**) and `cargo clippy --locked --all-targets -- -D warnings`. All 38 MFT tests passed. The ignored Shell-deletion integrations and saved-scan timing test were not enabled for this scanner repair.

## Build the candidate

Run the normal formatting, locked tests and strict Clippy checks. Commit the tested source first; the scripts reject a dirty checkout.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build_release.ps1
powershell -ExecutionPolicy Bypass -File scripts/package_release.ps1 -TestCandidate -OutputDirectory artifacts/candidates/v0.3.25
python scripts/check_brand_resources.py
```

`BUILD_INFO.json` records the exact source SHA, version, target, toolchain and executable checksum. `SHA256SUMS.txt` records the candidate's exact distribution files. The app and Windows resources derive version 0.3.25 from Cargo. The candidate ZIP includes the EXE, quick-start guide, MIT license, current notices and build provenance.

## Required manual acceptance on the failing drive

The automated Windows session has no administrator token. Synthetic tests do not establish that the user's failing drive completes a live scan. UAC error 740 is not a scan test.

1. Extract `artifacts/candidates/v0.3.25/Diskvale-windows-x64.zip` to a fresh folder, open `Diskvale.exe` and approve the normal administrator prompt. Confirm **0.3.25** in Ready/Scan notes; scanning must wait for your explicit action.
2. Select the **same NTFS drive that failed in v0.3.24**, click Scan and wait for the entire scan to finish. Confirm there is no record-0 error, later metadata error, cancellation or partial-result notice. Record elapsed time, file/folder counts and totals privately for comparison; do not treat them as benchmarks.
3. Navigate folders in Map, inspect Files and Types, and search for a known filename or extension.
4. Save a snapshot to a private local folder, reopen it and export a CSV. Confirm the reopened result is labeled Saved/Read only and deletion is disabled. Keep snapshots, CSVs and filenames private.
5. Check cancellation separately if convenient; a cancelled result must be marked partial and cannot authorize deletion. **No deletion test is needed.**

Report the drive letter/filesystem and whether the whole scan completed. If it fails, provide the exact error and version with sensitive paths redacted. Do not upload a full drive snapshot or raw MFT.

## Remaining limits

Lists must have a complete, unnamed mapping in their base attribute, with no compression, sparse runs or encryption, and fit within the stated safety bounds. Concurrently changed bootstrap metadata is rejected with a rescan instruction. Flat-MFT replay cannot supply external volume clusters. Existing experimental-reader, memory, accounting and non-atomic live-scan limits remain; see [technical details](technical.md).

Public release and announcement acceptance remain pending the real-drive result.
