<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/diskvale-logo.png">
  <img src="assets/brand/diskvale-logo-on-light.png" alt="Diskvale" width="300">
</picture>

**An open-source Windows disk explorer that helps you see what is taking up space.**

Diskvale turns a whole NTFS drive into an interactive map, with large-file lists, file-type totals and search. It runs locally as a native Rust/egui application.

**[Download v0.3.27 for Windows](https://github.com/colin-automates/Diskvale/releases/download/v0.3.27/Diskvale-windows-x64.zip)** · [Release notes](https://github.com/colin-automates/Diskvale/releases/tag/v0.3.27) · [Watch the demo](https://colin-automates.github.io/Diskvale/)

**Early preview · Windows 10/11 x64 · NTFS · Administrator required.** The build is unsigned; Windows may show an unknown-publisher warning.

![Diskvale exploring a generated studio drive](docs/media/diskvale.png)

## Download and run

| Download v0.3.27 | What you get |
| --- | --- |
| [Windows x64 portable ZIP](https://github.com/colin-automates/Diskvale/releases/download/v0.3.27/Diskvale-windows-x64.zip) | Recommended: Diskvale.exe, quick-start guide, build provenance, MIT license and third-party notices. |
| [Diskvale.exe](https://github.com/colin-automates/Diskvale/releases/download/v0.3.27/Diskvale.exe) | Standalone application, with no installer. |
| [SHA-256 checksums](https://github.com/colin-automates/Diskvale/releases/download/v0.3.27/SHA256SUMS.txt) | Checksums for the exact release assets. |
| [Source for this version](https://github.com/colin-automates/Diskvale/archive/refs/tags/v0.3.27.zip) | Source, tests, artwork and documentation at the release tag. |

1. Download the ZIP and extract it into a folder.
2. Open `Diskvale.exe` and approve the Windows administrator prompt.
3. Select an NTFS drive and click **Scan**. Launching does not start a scan automatically.
4. Explore **Map**, **Files** and **Types**. Use **Rescan** to refresh results.

No Rust, Python, Node.js or separate Visual C++ runtime installation is needed to run the download. Check its version in the Ready status, **Scan notes**, or Windows Properties → Details. `BUILD_INFO.json` identifies the source commit and toolchain. In PowerShell, `Get-FileHash .\Diskvale.exe -Algorithm SHA256` lets you compare the EXE with `SHA256SUMS.txt`.

## What you can do

- **See storage visually:** a nested treemap uses area for size and color for file category. Hover for details; double-click folders and use Backspace, Up or breadcrumbs to return.
- **Find large files and types:** sort the Files list by size or date; view extension totals in Types. Filters follow you across views and folders.
- **Search:** combine names, extensions, sizes and ages, such as `ext:mp4 >1GB` or `ext:zip older:1y`. Ctrl+K focuses search. Unicode lowercase matching is consistent across all views.
- **Save and compare:** save JSON snapshots, export CSV, or compare compatible captures of the same drive. Background jobs show progress, cancellation and errors.
- **Clean up deliberately:** right-click an individual item and choose **Delete…**, then confirm the exact path. Recycling is the default; refused recycling needs a separate confirmation for permanent deletion. Saved, partial and stale results cannot authorize deletion. Rescan after changes.

Sizes default to decimal GB/MB/KB; binary GiB/MiB/KiB are available. Size filters use the selected metric. See [size accounting, search units, snapshot compatibility and deletion safeguards](docs/technical.md).

## Early-preview limitations

The **experimental NTFS reader** scans whole local NTFS drives. Arbitrary folders, network shares and other filesystems are unsupported; navigating a folder explores the existing drive index. Supported nonresident attribute lists are resolved and validated for the MFT and ordinary files/directories. Split, compressed, sparse or encrypted list layouts remain unsupported. Incomplete metadata returns an explicit error rather than silently selecting another scanner. A parent-reference inconsistency permits one complete validated reread; persistent inconsistencies still fail. See the [supported layouts and limits](docs/technical.md#current-scope).

Large scans can require substantial memory. **Full-volume correctness and performance parity with other disk tools have not been established.** Scans are live metadata reads, not atomic filesystem snapshots. Hard links share allocation, and directory streams and volume overhead have specific accounting rules described in the [technical notes](docs/technical.md). Live USN updates and duplicate-file discovery are not implemented.

Deletion rechecks NTFS and volume identity, rejects reparse paths and holds ancestor handles during the Windows operation. These are **best-effort safeguards**, not atomic or race-free deletion. A folder deletion affects its current contents, including files added since scanning. Permanent deletion cannot be undone. Recycled files still occupy space until the Bin is emptied. Read the [detailed safety limits](docs/technical.md#snapshots-background-work-and-deletion) before using cleanup actions.

## Demo

[![Play the Diskvale demo](docs/media/demo-poster.png)](https://colin-automates.github.io/Diskvale/)

[Watch the 30-second video](https://colin-automates.github.io/Diskvale/) · [Download the MP4](https://github.com/colin-automates/Diskvale/releases/download/v0.3.27/Diskvale-Demo.mp4)

The screenshot and tour use the actual application with **generated drive data**. File counts and timings shown are illustrative, not real-drive benchmarks. The existing demo shows the same interface; v0.3.27 reduces scan allocation work and adds stage timings to Scan notes. See the [performance validation](docs/scan-performance-candidate.md). [Demo source and rendering instructions](tools/demo-video/README.md).

## Documentation and feedback

[How it works and safety details](docs/technical.md) · [Build from source and CLI usage](docs/build.md) · [Contributing](CONTRIBUTING.md) · [Release maintenance](docs/release.md) · [Regression validation](docs/review-fixes-2026-10-10.md)

**[Report a bug](https://github.com/colin-automates/Diskvale/issues/new?template=bug_report.yml)** with your Diskvale/Windows versions, drive context, reproduction steps and exact redacted error. Snapshots and CSVs contain filenames and paths. Use synthetic samples or redacted screenshots; do not attach full drive captures by default.

## License

Diskvale's original source, branding and demo use the **[MIT license](LICENSE)**. Dependencies and embedded fonts retain their own licenses in [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt), included in portable releases. Diskvale is an independent project and includes no WizTree code or binaries.
