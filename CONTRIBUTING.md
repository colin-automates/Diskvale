# Contributing to Diskvale

Bug reports, improvements to NTFS accounting, performance work, and UI refinements are welcome. Contributions are made under the project's MIT license.

## Development

Follow the Windows build instructions in the [README](README.md). Keep `Cargo.lock` committed and use `--locked` when checking a change.

```powershell
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Use synthetic fixtures for scanner and UI tests. The normal test suite needs no administrator token and does not scan a real drive. Ignored tests include Windows Recycle Bin/permanent-deletion integration checks that create disposable fixtures, plus a saved-scan timing test. Inspect a specific ignored test before running it; do not enable all ignored tests as part of routine CI.

To preview the UI with demo data from a standard terminal:

```powershell
python tools/demo-video/generate_data.py
cargo run --example map_review -- --snapshot artifacts/diskvale-demo/demo-data.json --compact
```

The Python generator uses only the standard library and does not create the indexed files. Building and running the demo video tooling is optional; see its [README](tools/demo-video/README.md).

## Changes and reports

Explain the user-visible problem, what changes, and how you verified it. Add a regression test when it protects a meaningful scanner, accounting, or interaction behavior. Include synthetic screenshots for visual changes and state the window size used.

Never commit personal scan results, raw MFT captures, CSV exports, credentials, desktop recordings, or development session logs. Snapshots and CSV exports can reveal complete file names and locations. Use generated demo data for public media.

Keep scanning read-only, cancellation explicit, and deletion confirmation tied to the exact target. Preserve target revalidation and the separate confirmation for permanent deletion. Do not replace an unsupported NTFS scan with an automatic folder-scan fallback.

## Release maintenance

Update the Cargo package version and use `scripts/build_release.ps1` to build with compiler path remapping. `scripts/package_release.ps1` creates the portable ZIP and checksums, including the MIT license and upstream notices. If dependencies change, run `python scripts/third_party_notices.py` to regenerate the notices before packaging; this may download missing upstream license files from the exact crate source commits.

Publish the executable, portable ZIP, checksums, standalone third-party notices, and demo MP4 on a GitHub Release tagged for that version. GitHub supplies source ZIP/tar archives for the tag. Avoid adding compiled executables or raw demo captures to the source tree.
