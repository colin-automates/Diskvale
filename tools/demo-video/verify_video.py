"""Verify the complete finished file and save a concise local QA record."""
import hashlib
import json
import os
import shutil
import re
import subprocess
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "artifacts" / "diskvale-demo"
OUT = SOURCE / "short"
FF = os.environ.get("DISKVALE_FFMPEG") or shutil.which("ffmpeg")
if not FF:
    raise SystemExit("Install FFmpeg on PATH or set DISKVALE_FFMPEG to its executable.")
VIDEO = OUT / "Diskvale-Demo-Short.mp4"
soundtrack = json.loads((OUT / "audio-manifest.json").read_text(encoding="utf-8"))
music = json.loads((ROOT / "assets/audio/soundtrack.json").read_text(encoding="utf-8"))
assert soundtrack["music"] == music
assert hashlib.sha256((ROOT / "assets/audio" / music["excerpt_file"]).read_bytes()).hexdigest() == music["excerpt_sha256"]
assert hashlib.sha256(VIDEO.read_bytes()).hexdigest() == soundtrack["video_sha256"]
clicks = json.loads((ROOT / "assets/audio/clicks.json").read_text(encoding="utf-8"))
assert soundtrack["clicks"] == clicks
assert soundtrack["added_click_effects"] == len(clicks["cues"]) == 4
assert soundtrack["added_synthesized_score"] is False
for recording in clicks["files"]:
    assert hashlib.sha256((ROOT / "assets/audio" / recording["file"]).read_bytes()).hexdigest() == recording["sha256"]
for file, key in (("music-bed.wav", "music_bed_sha256"), ("recorded-clicks.wav", "click_stem_sha256")):
    assert hashlib.sha256((OUT / file).read_bytes()).hexdigest() == soundtrack[key]
capture = json.loads((SOURCE / "capture-manifest.json").read_text(encoding="utf-8"))
assert capture["fps"] == 30 and capture["frames"] == 1260
assert len(capture["milestones"]) == 7
for text in capture["painted_text"]:
    assert not any(term in text.lower() for term in ("c:\\users\\", "c:/users/", "\\appdata\\", "/appdata/"))
scan = json.loads((SOURCE / "demo-data.json").read_text(encoding="utf-8"))
assert scan["root"] == "Z:\\"
assert "DISKVALE_SYNTHETIC_DEMO_ONLY" in scan["stats"]["notices"]
assert scan["stats"]["files"] == 2929
edit = json.loads((OUT / "edit-manifest.json").read_text(encoding="utf-8"))
assert edit["frames"] == 900 and edit["duration_seconds"] == 30
assert edit["chapter_numbers"] == [str(i) for i in range(1, 7)]
assert edit["long_fictional_data_sentence"] is False
assert len(edit["source_frame_indices"]) == 750
assert all(0 <= frame < capture["frames"] for frame in edit["source_frame_indices"])
assert all(a < b for a,b in zip(edit["source_frame_indices"],edit["source_frame_indices"][1:]))
assert len(soundtrack["click_events"]) == 4
for event, cue in zip(soundtrack["click_events"], clicks["cues"]):
    frame = round(cue["capture_seconds"] * capture["fps"])
    video_frame = 75 + edit["source_frame_indices"].index(frame)
    assert event["capture_frame"] == frame and event["video_frame"] == video_frame
    assert event["press_seconds"] == video_frame / 30
    assert event["release_seconds"] == (video_frame + 1) / 30

# The isolated click stem must contain sound only at the four visible actions.
decoded_clicks = subprocess.run([FF, "-v", "error", "-i", str(OUT / "recorded-clicks.wav"),
                                "-f", "f32le", "-"], capture_output=True, check=True)
click_samples = np.frombuffer(decoded_clicks.stdout, dtype="<f4").reshape(-1, 2)
assert len(click_samples) == 30 * 48000
allowed = np.zeros(len(click_samples), dtype=bool)
for event in soundtrack["click_events"]:
    start = round(event["press_seconds"] * 48000)
    end = start + round(.12 * 48000)
    allowed[start:end] = True
    assert np.max(np.abs(click_samples[start:end])) > .1
assert np.max(np.abs(click_samples[~allowed])) == 0

decoded = subprocess.run([FF, "-nostdin", "-v", "error", "-nostats", "-progress", "pipe:1",
                          "-i", str(VIDEO), "-map", "0:v:0", "-map", "0:a:0", "-f", "null", "NUL"],
                         capture_output=True, text=True, check=True)
assert not decoded.stderr.strip(), decoded.stderr
assert int(re.findall(r"^frame=(\d+)$", decoded.stdout, re.M)[-1]) == 900
info = subprocess.run([FF, "-hide_banner", "-i", str(VIDEO)], capture_output=True, text=True).stderr
assert "1920x1080" in info and "30 fps" in info and "Video: h264" in info
assert "Audio: aac" in info and "48000 Hz, stereo" in info
assert "Duration: 00:00:30.00" in info
audio = subprocess.run([FF, "-nostdin", "-hide_banner", "-i", str(VIDEO), "-vn", "-af",
                        f"loudnorm=I={soundtrack['target_lufs']}:TP=-2:LRA=9:print_format=json", "-f", "null", "NUL"],
                       capture_output=True, text=True, check=True)
measurement = json.loads(audio.stderr[audio.stderr.rfind("{ "):audio.stderr.rfind("}") + 1]
                         if "{ " in audio.stderr else audio.stderr[audio.stderr.rfind("{"):audio.stderr.rfind("}") + 1])
assert float(measurement["input_tp"]) < -1
bed_measurement = subprocess.run([FF, "-nostdin", "-hide_banner", "-i", str(OUT / "music-bed.wav"), "-af",
                                  f"loudnorm=I={soundtrack['target_lufs']}:TP=-2:LRA=9:print_format=json",
                                  "-f", "null", "NUL"], capture_output=True, text=True, check=True)
bed_levels = json.loads(bed_measurement.stderr[bed_measurement.stderr.rfind("{"):bed_measurement.stderr.rfind("}")+1])
assert abs(float(bed_levels["input_i"]) - soundtrack["target_lufs"]) < 1
assert abs(float(measurement["input_i"]) - soundtrack["target_lufs"]) < 1
subprocess.run([FF, "-y", "-v", "error", "-i", str(VIDEO), "-vf", "fps=1/3,scale=640:360,tile=5x2",
                "-frames:v", "1", str(OUT / "encoded-contact-sheet.png")], check=True)
report = dict(file=VIDEO.name, duration_seconds=30, resolution=[1920, 1080], fps=30,
              codecs=["H.264", "AAC stereo 48 kHz"], frames_fully_decoded=900,
              size_bytes=VIDEO.stat().st_size, sha256=hashlib.sha256(VIDEO.read_bytes()).hexdigest(),
              decode_errors=[], privacy=dict(source="Generated fictional drive index",
              desktop_pixels="Excluded by framebuffer capture", unique_text_strings_audited=len(capture["painted_text"]),
              personal_path_hits=0, real_scans=0, indexed_file_operations=0, synthetic_file_count=2929),
              interaction_checkpoints_in_source=7, features_in_edit=6, chapter_numbers=edit["chapter_numbers"],
              long_fictional_data_sentence=False, background=edit["background"],audio_lufs=float(measurement["input_i"]),
              audio_true_peak_db=float(measurement["input_tp"]), music_bed_lufs=float(bed_levels["input_i"]),
              recorded_click_events_verified=4, soundtrack=soundtrack)
(OUT / "verification.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
print(json.dumps(report, indent=2))
