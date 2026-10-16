# External drive validation: v0.3.28

v0.3.28 adds explicit exFAT/FAT32 directory scans and detects ready fixed and removable disks. The requested E: drive is exFAT. NTFS retains the validated MFT reader and never falls back after an NTFS read or validation failure. Drive discovery and refresh run off the UI thread; selection alone does not start a scan.

exFAT/FAT32 scanning reads directory and file metadata, including Windows-reported file allocation. It does not read file contents, follow symlinks or reparse directories, or enable deletion. Folder allocation and filesystem metadata are excluded. Cancelled or error-bearing scans remain partial, and a final volume serial/filesystem/capacity check rejects detected drive changes or disappearance.

## Validation on 2026-10-16

- `cargo fmt --all -- --check`: passed.
- `cargo test --locked`: 150 library tests and 7 integration tests passed; the two destructive integrations, saved-scan benchmark, and live external-drive check remain ignored in ordinary runs. Filesystem fixtures required execution outside the restricted sandbox, which denied native rename and hard-link calls.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- New regressions cover exFAT/FAT routing, retaining NTFS errors without fallback, ready removable/fixed drive types, read-only runtime provenance, cancellation, exFAT snapshot save/reopen, and selecting a refreshed exFAT drive without starting a scan.
- The individually selected read-only E: check reached 68,184 files and 1,707 folders in 30.02 seconds, with zero read errors and zero unknown allocations, then cancelled cleanly at its requested time bound. It did not write to E:. This validates real exFAT metadata reads and cancellation, not a completed scan or an independent correctness comparison.
- The rendered production UI was inspected with generated data at 1050×700 and 1400×900. Both show the exFAT filesystem correctly; these captures contain no E: filenames or desktop content.
- A Windows x64 candidate was built with a static C runtime and compiler path remapping. PE checks passed for version 0.3.28, the administrator manifest, embedded icon sizes, and absence of a separate Visual C++ runtime requirement. The executable does not contain the local profile path.

## Full-drive acceptance

On 2026-10-16 the user confirmed that a full scan of the same E: drive worked on the v0.3.28 candidate, then authorized publication. The candidate EXE SHA-256 is `4e2bcc2af7696fe8adb80c2975224a37e7e1b9baec70f8ae8935e660ae0af645`. This is user-confirmed completion, not an independent comparison of every file or its totals. No private drive capture, filename list or scan export was collected for publication.

The accepted candidate, aggregate test result, source fingerprints, and generated UI captures are under the ignored `artifacts/external-drive-candidate/` directory. That candidate was built with uncommitted source changes, explicitly recorded in `CANDIDATE_INFO.json`. Public release assets are rebuilt and packaged from the clean release commit containing the same application code plus release documentation; their `BUILD_INFO.json` records the source, toolchain and EXE checksum. Older published tags and assets retain their original identities. Physical unplug/reconnect during scanning and a real FAT32 device remain unverified.
