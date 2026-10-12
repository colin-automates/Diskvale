# Build Diskvale from source

[User guide](../README.md) · [Contributing](../CONTRIBUTING.md)


Install [Rust through rustup](https://rustup.rs/) and [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio) with **Desktop development with C++** and a **Windows SDK**. `rust-toolchain.toml` selects stable Rust for Windows MSVC; the minimum supported Rust version is 1.88.

```powershell
git clone https://github.com/colin-automates/Diskvale.git
cd Diskvale
powershell -ExecutionPolicy Bypass -File scripts/build_release.ps1
powershell -ExecutionPolicy Bypass -File scripts/package_release.ps1
```

`canopy` is the historical internal crate/binary name. The product, executable branding, and window title are Diskvale. The Windows SDK resource compiler embeds the app icon, version information, and administrator manifest. All application artwork is embedded in the executable.

The release script builds the Windows x64 target with a static C runtime and compiler path remapping. Published executables need no separate Visual C++ runtime and do not contain the build machine's profile or checkout paths. An ordinary local `cargo build --release --locked` also builds the application, but uses Cargo's default runtime linkage.

Builds and ordinary tests work from a standard terminal. Launching the production app, including debug builds, requires administrator access. Use an administrator terminal for `cargo run` or CLI scans.

```powershell
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Tests cover synthetic NTFS records, accounting, queries, layout, cached results, and deletion confirmation/revalidation. Tests that perform Windows deletion on disposable fixtures and the large saved-scan timing check are ignored by default. See [CONTRIBUTING.md](../CONTRIBUTING.md).

Release scripts require a clean Git checkout of the committed source. Commit relevant changes before building; they record the source commit, toolchain, version and executable hash in `BUILD_INFO.json`. Packaging rejects a different commit, modified checkout or mismatched executable.

To package a portable release after building:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package_release.ps1
```

The package, standalone executable, build provenance, and checksums are written under `artifacts/release/`. Building the application does not require the optional Python/Node demo or artwork tools.

### Command-line examples

From an administrator terminal:

```powershell
.\Diskvale.exe --path "C:\" --compact
.\Diskvale.exe --path "C:\" --auto-scan
.\Diskvale.exe --scan "C:\" --json scan.json --csv files.csv > scan-summary.json
.\Diskvale.exe --snapshot scan.json --view largest
```

`--compact` uses a 1050×700 window; `--view largest` and `--view types` choose a results tab. `--focus "Projects"` focuses a relative path within loaded results. The release uses the Windows GUI subsystem; redirect output for headless scans. `--mft-file saved.mft` is a diagnostic parser input; `--snapshot scan.json` opens saved results. The production executable still requires an administrator token for these modes.


See [release maintenance](release.md) for publication and public-download verification. Download the current [v0.3.26 early preview](https://github.com/colin-automates/Diskvale/releases/tag/v0.3.26).
