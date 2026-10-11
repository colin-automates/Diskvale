# Contributing to Diskvale

Bug reports, improvements to NTFS accounting, performance work, and UI refinements are welcome. Contributions are made under the project's MIT license.

## Development

Follow the [Windows build instructions](docs/build.md). Keep `Cargo.lock` committed and use `--locked` when checking a change.

```powershell
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Use synthetic fixtures for scanner and UI tests. The normal test suite needs no administrator token and does not scan a real drive. Ignored tests include Windows Recycle Bin/permanent-deletion integration checks that create disposable fixtures, plus a saved-scan timing test. Inspect a specific ignored test before running it; do not enable all ignored tests as part of routine CI.

Real Shell deletion integration additionally requires a separate explicit opt-in, `DISKVALE_DELETE_FIXTURE_TESTS=1`. Only after that opt-in, run one inspected test by its exact name:

```powershell
$env:DISKVALE_DELETE_FIXTURE_TESTS = '1'
cargo test --locked --lib recycle::tests::shell_recycles_only_new_disposable_file_and_folder_fixtures -- --exact --ignored --nocapture
# Permanent deletion needs its own intentional selection:
cargo test --locked --lib recycle::tests::shell_permanently_deletes_only_new_disposable_file_and_folder_fixtures -- --exact --ignored --nocapture
Remove-Item Env:DISKVALE_DELETE_FIXTURE_TESTS
```

These tests create unique disposable folders under this checkout's `artifacts` directory and check confinement before operation. They never empty the Recycle Bin. Normal identity, pre-operation replacement, callback-veto, staged-output failure, generation, background responsiveness, stream-accounting, and sidebar-page regressions run without this opt-in and do not call Shell deletion. See [the review validation report](docs/review-fixes-2026-10-10.md) for executed checks and measured limits.

To preview the UI with demo data from a standard terminal:

```powershell
python tools/demo-video/generate_data.py
cargo run --example map_review -- --snapshot artifacts/diskvale-demo/demo-data.json --compact
```

The Python generator uses only the standard library and does not create the indexed files. Building and running the demo video tooling is optional; see its [README](tools/demo-video/README.md).

## Changes and reports

Use the [bug-report form](https://github.com/colin-automates/Diskvale/issues/new?template=bug_report.yml). Include Diskvale and Windows versions, drive/filesystem context, reproduction steps, expected/actual behavior, and exact redacted error text or a screenshot. Do not upload full drive captures by default.

Explain the user-visible problem, what changes, and how you verified it. Add a regression test when it protects a meaningful scanner, accounting, or interaction behavior. Include synthetic screenshots for visual changes and state the window size used.

Never commit personal scan results, raw MFT captures, CSV exports, credentials, desktop recordings, or development session logs. Snapshots and CSV exports can reveal complete file names and locations. Use generated demo data for public media.

Keep scanning read-only, cancellation explicit, and deletion confirmation tied to the exact target. Preserve target revalidation and the separate confirmation for permanent deletion. Do not replace an unsupported NTFS scan with an automatic folder-scan fallback.

## Release maintenance

Update the Cargo package version and its lockfile entry, commit the source, then use `scripts/build_release.ps1` from a clean checkout to build with compiler path remapping. It records source commit, toolchain, version and EXE checksum. `scripts/package_release.ps1` checks that provenance and creates the portable ZIP and checksums, including the MIT license and upstream notices. If dependencies change, run `python scripts/third_party_notices.py` to regenerate the notices before packaging; this may download missing upstream license files from the exact crate source commits.

Publish the executable, portable ZIP, checksums, standalone third-party notices, and demo MP4 on a GitHub Release tagged for that version. GitHub supplies source ZIP/tar archives for the tag. Avoid adding compiled executables or raw demo captures to the source tree.

See [release maintenance](docs/release.md) for the complete gates. Experimental previews are prereleases; their README, website and quick-start links use the explicit tag, since GitHub's latest-release redirect excludes prereleases.
