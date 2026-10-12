# Release maintenance

Diskvale is an early preview. Publish experimental builds as GitHub prereleases with explicit versioned download links; [GitHub excludes prereleases from latest](https://docs.github.com/en/rest/releases/releases#get-the-latest-release). Do not move an existing tag or make a preview stable to change that redirect.

## Code signing policy

See the [code signing policy](../CODE_SIGNING.md) and [SignPath setup](code-signing-setup.md). Existing 0.3.27 downloads are unsigned; Foundation approval and account configuration are pending. After setup, use **Prepare signed Windows release** from `main` for a new version, with a manual SignPath approval. Verify its signed artifact and use all assets from that run together. Signing changes the EXE hash, so provenance, ZIP and checksums must be generated after signature verification. Do not replace an existing release's unsigned assets or call a dry run signed.

## Publication checks

1. Fetch refs, preserve existing work, inspect applicable repository instructions and branch rules. Choose an unused version. Update `Cargo.toml`, the `canopy` lockfile entry, README/page links, and release notes together. The app and Windows resources derive their version from Cargo.
2. Run `cargo fmt --all -- --check`, `cargo test --locked`, and `cargo clippy --locked --all-targets -- -D warnings`. Normal tests use synthetic/disposable fixtures; inspect and deliberately select any ignored test. Never run all ignored deletion tests blindly.
3. Commit relevant source and documentation through the permitted repository workflow. From that **clean commit**, run:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts/build_release.ps1
   powershell -ExecutionPolicy Bypass -File scripts/package_release.ps1
   python scripts/check_brand_resources.py
   ```

   The build records `BUILD_INFO.json` beside the EXE. Packaging checks clean source, commit, version and EXE hash before including it in the ZIP and standalone assets. Windows CI uploads the complete package from its checked-out commit. Use one verified run's artifacts together. Do not relabel old binaries.
4. Verify icons, administrator manifest, versions, runtime imports, license/notices, quick-start instructions and archive contents. Confirm the standalone EXE matches the ZIP's EXE. Extract into a fresh directory and smoke-test it where Windows access allows. Validate startup with no automatic scan, a read-only NTFS fixture scan, navigation/search and save/open/export/compare. Record unavailable tests honestly. Destructive tests must use newly created disposable fixtures only.
5. Tag the tested commit without overwriting existing tags. Prepare a **draft prerelease** and upload `Diskvale.exe`, `Diskvale-windows-x64.zip`, `SHA256SUMS.txt`, `THIRD_PARTY_NOTICES.txt`, `BUILD_INFO.json`, and `Diskvale-Demo.mp4` while the demo is advertised. Check source SHA, CI, assets and checksums before publishing. Do not publish if a critical safety or build check fails.
6. After publication, resolve the tag to the tested SHA. Download public ZIP/EXE/checksums into a fresh directory; compare every asset hash and the EXE inside the ZIP, inspect resources again, and launch where supported. Verify deployed Pages separately: its main button must reach the intended preview. Audit README, page, quick start and direct links; do not rely on `/releases/latest` for a prerelease.

Keep build outputs, personal scan data, raw MFT captures, secrets and private logs out of commits. Release notes must distinguish synthetic tests from interactive Windows/full-volume checks and describe known compatibility and deletion limits. See [contributing](../CONTRIBUTING.md), [technical notes](technical.md), and [the review-fix validation](review-fixes-2026-10-10.md).
