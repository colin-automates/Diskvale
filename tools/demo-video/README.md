# Diskvale product demo

The 30-second demo records the actual production egui UI against a generated `Z:\` studio drive. It demonstrates the map, folder navigation and inspection, largest files, `ext:mp4 >1GB`, extension totals, and media isolation. Source motion retains its original speed; idle time and return navigation are cut.

`generate_data.py` creates a deterministic 2,929-file, 33-folder index without reading or creating any of the indexed files. `examples/demo_video.rs` includes the production UI, requires the synthetic fixture marker and a standard Windows token, removes the live drive menu, discards physical input, and records only the egui framebuffer. It asserts seven interaction checkpoints and audits painted text. It does not capture the desktop or perform indexed file operations.

`build_video.py` edits the 42-second UI capture into 25 seconds of UI plus a 2.5-second opening and ending. It adds captions numbered 1–6, a full-resolution dithered background, and an original synthesized instrumental score. The final video is 1920×1080 at 30 fps, H.264/AAC, with metadata stripped and fast-start playback enabled.

## Reproduce on Windows

Install Python 3.10+, Rust, and FFmpeg. Put `ffmpeg.exe` on PATH, or set `DISKVALE_FFMPEG` to its full path. NumPy and Pillow are needed for editing and the soundtrack; the fixture generator itself uses only Python's standard library. Windows Segoe UI fonts are used for captions.

Run from the repository root in a standard terminal:

```powershell
python -m pip install -r tools/demo-video/requirements.txt
python tools/demo-video/generate_data.py
cargo build --example demo_video --locked
$demoRoot = Join-Path $PWD 'artifacts/diskvale-demo'
$ffmpeg = if ($env:DISKVALE_FFMPEG) { $env:DISKVALE_FFMPEG } else { (Get-Command ffmpeg).Source }
target/debug/examples/demo_video.exe --snapshot "$demoRoot/demo-data.json" --output "$demoRoot" --ffmpeg "$ffmpeg" --fps 30
python tools/demo-video/build_video.py
python tools/demo-video/verify_video.py
```

The display must accommodate a 1600×900 client framebuffer. Capture frames, intermediate media, edit manifests, and verification reports are local outputs under the ignored `artifacts/diskvale-demo/` directory.

The finished export is `artifacts/diskvale-demo/short/Diskvale-Demo-Short.mp4`. The published copy is [docs/media/diskvale-demo.mp4](../../docs/media/diskvale-demo.mp4). Scanning speed and deletion outcomes are not simulated in the tour.
