"""Edit genuine Diskvale UI footage into a local 1080p product demo."""
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "artifacts" / "diskvale-demo"
OUT = SOURCE / "short"
OUT.mkdir(parents=True, exist_ok=True)
FF = os.environ.get("DISKVALE_FFMPEG") or shutil.which("ffmpeg")
if not FF:
    raise SystemExit("Install FFmpeg on PATH or set DISKVALE_FFMPEG to its executable.")
W, H, FPS = 1920, 1080, 30
INTRO, TOUR, OUTRO = 2.5, 25, 2.5
TOTAL = INTRO + TOUR + OUTRO
EXPORT_NAME = "Diskvale-Demo-Short.mp4"
AUDIO_ROOT = ROOT / "assets" / "audio"
AUDIO_LUFS, AUDIO_TRUE_PEAK = -26, -2
MINT, WHITE, MUTED = "#8BE5D0", "#E6EDEA", "#98ADB7"
FONTS = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts"


def font(size, bold=False):
    return ImageFont.truetype(str(FONTS / ("segoeuib.ttf" if bold else "segoeui.ttf")), size)


def background():
    yy, xx = np.mgrid[0:H, 0:W]
    light = np.exp(-(((xx - 1560) / 780) ** 2 + ((yy - 140) / 730) ** 2))
    # Quantizing a smooth dark gradient directly to 8-bit produces visible rings.
    # Render the field at full resolution in float, then dither each colour channel.
    # The fixed, fine noise breaks up those contours without an animated texture.
    noise = np.random.default_rng(20261009).uniform(-.8, .8, (H, W, 3))
    base = np.empty((H, W, 3), dtype=np.uint8)
    for i, value in enumerate((13, 20, 25)):
        field = value + light * (5, 14, 17)[i] + noise[:, :, i]
        base[:, :, i] = np.clip(np.rint(field), 0, 255).astype(np.uint8)
    result = Image.fromarray(base)
    d = ImageDraw.Draw(result)
    d.line((152, 64, 1768, 64), fill="#263C46", width=1)
    return result


BG = background()
logo = Image.open(ROOT / "assets" / "brand" / "diskvale-logo.png").convert("RGBA")
logo = logo.resize((642, 207), Image.Resampling.LANCZOS)
preview = Image.open(SOURCE / "frame-0000.ppm").convert("RGB")
preview_small = preview.resize((824, 464), Image.Resampling.LANCZOS)


def badge(image, x=1600, y=20):
    d = ImageDraw.Draw(image)
    d.rounded_rectangle((x, y, x + 167, y + 36), radius=18, fill="#1C333A", outline="#36575E")
    d.ellipse((x + 14, y + 14, x + 22, y + 22), fill=MINT)
    d.text((x + 33, y + 4), "Demo data", font=font(20), fill=WHITE)


def intro(t):
    image = BG.copy()
    d = ImageDraw.Draw(image)
    image.paste(logo, (130, 222), logo)
    d.text((158, 463), "Know what’s taking", font=font(58, True), fill=WHITE)
    d.text((158, 538), "up space.", font=font(58, True), fill=MINT)
    d.text((161, 637), "A clearer view of your drive.", font=font(28), fill=MUTED)
    d.text((160, 932), "DISK EXPLORER FOR WINDOWS", font=font(19, True), fill=MUTED)
    shift = int(45 * (1 - min(t / 1.1, 1)) ** 3)
    px, py = 962 + shift, 329
    d.rounded_rectangle((px - 2, py - 47, px + 826, py + 466), radius=16,
                        fill="#1C2931", outline="#3C5B65", width=2)
    d.ellipse((px + 16, py - 30, px + 24, py - 22), fill=MINT)
    d.text((px + 37, py - 40), "STUDIO DRIVE", font=font(18, True), fill=WHITE)
    mask = Image.new("L", (824, 464), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, 823, 463), radius=10, fill=255)
    image.paste(preview_small, (px, py), mask)
    badge(image)
    return image


CHAPTERS = [
    (0, 4, "1", "Your drive at a glance", "See what’s taking up space."),
    (4, 8.3, "2", "Explore any folder", "Open folders. Inspect name, size and location."),
    (8.3, 12.2, "3", "Find the biggest files", "Browse your largest files."),
    (12.2, 16.9, "4", "Search with precision", "Find large videos with ext:mp4 >1GB."),
    (16.9, 21, "5", "Understand your file types", "Compare space used by each file type."),
    (21, 25, "6", "Focus on what matters", "Isolate a category on the map."),
]
# Select the useful interactions and shorten holds; pointer motion keeps its
# original speed. Every source frame comes from the audited synthetic UI capture.
SOURCE_RANGES = [(1.3, 5.3), (6.5, 10.8), (14.4, 18.3),
                 (20, 24.7), (27, 31.1), (34.4, 38.4)]


def source_frame_for(tour_frame_index):
    offset = 0
    for start, end in SOURCE_RANGES:
        count = round((end - start) * FPS)
        if tour_frame_index < offset + count:
            return round(start * FPS) + tour_frame_index - offset
        offset += count
    raise ValueError("Tour frame is outside the edit")
chapter_bases = []
for start, end, number, title, caption in CHAPTERS:
    image = BG.copy()
    d = ImageDraw.Draw(image)
    d.text((154, 17), number, font=font(26, True), fill=MINT)
    d.text((195, 15), title, font=font(28, True), fill=WHITE)
    d.text((160, 1016), caption, font=font(25), fill=WHITE)
    d.rounded_rectangle((154, 74, 1766, 986), radius=8, fill="#10161B", outline="#35505A", width=2)
    badge(image)
    chapter_bases.append(image)


def tour_frame(ui, t):
    index = next(i for i, chapter in enumerate(CHAPTERS) if chapter[0] <= t < chapter[1])
    image = chapter_bases[index].copy()
    image.paste(Image.fromarray(ui), (160, 80))
    d = ImageDraw.Draw(image)
    progress = t / TOUR
    d.line((160, 1064, 160 + int(1600 * progress), 1064), fill=MINT, width=3)
    return image


def outro(t):
    image = BG.copy()
    d = ImageDraw.Draw(image)
    # A soft, fictional treemap texture behind the brand card.
    texture = preview.crop((293, 140, 1585, 780)).resize((W, H), Image.Resampling.LANCZOS)
    texture = texture.filter(ImageFilter.GaussianBlur(12))
    field = .925 * np.asarray(BG, dtype=np.float32) + .075 * np.asarray(texture, dtype=np.float32)
    # Dither the blend too, rather than recreating banding with an 8-bit blend.
    field += np.random.default_rng(20261010).uniform(-.5, .5, field.shape)
    image = Image.fromarray(np.clip(np.rint(field), 0, 255).astype(np.uint8))
    image.paste(logo, (638, 308), logo)
    d = ImageDraw.Draw(image)
    text = "See your space clearly."
    width = d.textlength(text, font=font(48, True))
    d.text(((W - width) / 2, 557), text, font=font(48, True), fill=WHITE)
    text = "Map   /   Files   /   Types"
    width = d.textlength(text, font=font(26))
    d.text(((W - width) / 2, 647), text, font=font(26), fill=MINT)
    text = "Diskvale · Disk explorer for Windows"
    width = d.textlength(text, font=font(22))
    d.text(((W - width) / 2, 835), text, font=font(22), fill=MUTED)
    badge(image)
    return image


def read_exact(pipe, count):
    parts, length = [], 0
    while length < count:
        data = pipe.read(count - length)
        if not data:
            raise RuntimeError(f"Capture ended after {length} of {count} bytes")
        parts.append(data)
        length += len(data)
    return b"".join(parts)


def finish_audio():
    """Mix quiet CC0 music with recorded mouse presses at visible UI actions."""
    music = json.loads((AUDIO_ROOT / "soundtrack.json").read_text(encoding="utf-8"))
    track = AUDIO_ROOT / music["excerpt_file"]
    if hashlib.sha256(track.read_bytes()).hexdigest() != music["excerpt_sha256"]:
        raise ValueError("Soundtrack does not match its source manifest")
    fade_in, fade_out = 1.3, 2.5
    bed = (f"atrim=duration={TOTAL},asetpts=PTS-STARTPTS,"
           f"afade=t=in:st=0:d={fade_in},afade=t=out:st={TOTAL-fade_out}:d={fade_out}")
    target = f"loudnorm=I={AUDIO_LUFS}:TP={AUDIO_TRUE_PEAK}:LRA=9"
    measured = subprocess.run([FF, "-nostdin", "-hide_banner", "-i", str(track),
                               "-af", f"{bed},{target}:print_format=json", "-f", "null", "-"],
                              capture_output=True, text=True, check=True)
    levels = json.loads(measured.stderr[measured.stderr.rfind("{"):measured.stderr.rfind("}")+1])
    # Normalize only the music. Normalizing the finished mix would raise the bed
    # again when the short mouse transients are added.
    normalizer = (f"{target}:measured_I={levels['input_i']}:measured_TP={levels['input_tp']}:"
                  f"measured_LRA={levels['input_lra']}:measured_thresh={levels['input_thresh']}:"
                  f"offset={levels['target_offset']}:linear=true")
    music_bed = OUT / "music-bed.wav"
    subprocess.run([FF, "-y", "-v", "error", "-i", str(track), "-af", f"{bed},{normalizer}",
                    "-ar", "48000", "-ac", "2", "-c:a", "pcm_s24le", str(music_bed)], check=True)

    def decode(path, channels, filters=None):
        command = [FF, "-v", "error", "-i", str(path)]
        if filters:
            command += ["-af", filters]
        result = subprocess.run(command + ["-ar", "48000", "-ac", str(channels), "-f", "f32le", "-"],
                                capture_output=True, check=True)
        return np.frombuffer(result.stdout, dtype="<f4").reshape(-1, channels)

    music_samples = decode(music_bed, 2)
    samples = round(TOTAL * 48000)
    if len(music_samples) != samples:
        raise ValueError("Music must cover the complete 30-second edit")
    clicks = json.loads((AUDIO_ROOT / "clicks.json").read_text(encoding="utf-8"))
    recordings = {}
    for recording in clicks["files"]:
        path = AUDIO_ROOT / recording["file"]
        if hashlib.sha256(path.read_bytes()).hexdigest() != recording["sha256"]:
            raise ValueError("Mouse recording does not match its source manifest")
        sound = decode(path, 1, f"highpass=f={clicks['highpass_hz']},lowpass=f={clicks['lowpass_hz']}")
        peak = float(np.max(np.abs(sound)))
        if not peak:
            raise ValueError("Mouse recording is silent")
        target_peak = clicks[f"{recording['action']}_peak_dbfs"]
        recordings[recording["action"]] = sound * (10 ** (target_peak / 20) / peak)

    # The edit's source-frame mapping keeps cues on retained clicks. The press
    # and release are one frame apart, exactly as in the capture's input events.
    source_frames = [source_frame_for(i) for i in range(round(TOUR * FPS))]
    click_samples = np.zeros_like(music_samples)
    events = []
    for cue in clicks["cues"]:
        capture_frame = round(cue["capture_seconds"] * FPS)
        video_frame = round(INTRO * FPS) + source_frames.index(capture_frame)
        if source_frames[source_frames.index(capture_frame) + 1] != capture_frame + 1:
            raise ValueError("The edit cuts away the mouse release")
        for action, frame_offset in (("press", 0), ("release", 1)):
            sound = recordings[action]
            start = round((video_frame + frame_offset) * 48000 / FPS)
            if start + len(sound) > samples:
                raise ValueError("Mouse recording extends beyond the edit")
            click_samples[start:start+len(sound)] += sound
        events.append(dict(action=cue["action"], capture_frame=capture_frame, video_frame=video_frame,
                           press_seconds=video_frame / FPS, release_seconds=(video_frame + 1) / FPS))
    mixed = music_samples + click_samples
    if float(np.max(np.abs(mixed))) >= 1:
        raise ValueError("Soundtrack would clip; lower the mix levels")

    def save_wave(path, audio):
        subprocess.run([FF, "-y", "-v", "error", "-f", "f32le", "-ar", "48000", "-ac", "2", "-i", "-",
                        "-c:a", "pcm_s24le", str(path)], input=audio.astype("<f4").tobytes(), check=True)

    click_stem = OUT / "recorded-clicks.wav"
    mixed_track = OUT / "mixed-soundtrack.wav"
    save_wave(click_stem, click_samples)
    save_wave(mixed_track, mixed)
    video = OUT / EXPORT_NAME
    subprocess.run([FF, "-y", "-v", "error", "-i", str(OUT / "picture.mp4"),
                    "-i", str(mixed_track), "-map", "0:v:0", "-map", "1:a:0",
                    "-c:v", "copy", "-c:a", "aac", "-ar", "48000",
                    "-ac", "2", "-b:a", "192k", "-t", str(TOTAL),
                    "-map_metadata", "-1", "-movflags", "+faststart", str(video)], check=True)
    audio = dict(music=music, target_lufs=AUDIO_LUFS, fade_in_seconds=fade_in,
                 fade_out_seconds=fade_out, added_click_effects=len(events), added_synthesized_score=False,
                 clicks=clicks, click_events=events,
                 music_bed_sha256=hashlib.sha256(music_bed.read_bytes()).hexdigest(),
                 click_stem_sha256=hashlib.sha256(click_stem.read_bytes()).hexdigest(),
                 video_sha256=hashlib.sha256(video.read_bytes()).hexdigest())
    (OUT / "audio-manifest.json").write_text(json.dumps(audio, indent=2), encoding="utf-8")
    print(f"Finished {EXPORT_NAME}", flush=True)


def main():
    manifest = json.loads((SOURCE / "capture-manifest.json").read_text(encoding="utf-8"))
    assert manifest["frames"] == FPS * 42 and manifest["fps"] == FPS
    assert sum(round((end-start)*FPS) for start,end in SOURCE_RANGES) == round(TOUR*FPS)
    reader = subprocess.Popen([FF, "-v", "error", "-i", str(SOURCE / "ui-capture.mp4"),
                               "-f", "rawvideo", "-pix_fmt", "rgb24", "-"], stdout=subprocess.PIPE)
    writer = subprocess.Popen([FF, "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
                               "-s", f"{W}x{H}", "-r", str(FPS), "-i", "-", "-an",
                               "-c:v", "libx264", "-preset", "slow", "-tune", "grain", "-crf", "14", "-pix_fmt", "yuv420p",
                               str(OUT / "picture.mp4")], stdin=subprocess.PIPE)
    decoded_index = -1
    ui = None
    last = None
    intro_card = intro(INTRO)
    outro_card = outro(OUTRO)
    intro(2).save(OUT / "intro-design.png")
    outro_card.save(OUT / "outro-design.png")
    BG.save(OUT / "background.png")
    edit = dict(duration_seconds=TOTAL, fps=FPS, frames=round(TOTAL*FPS),
                source_ranges=SOURCE_RANGES, chapters=CHAPTERS,
                source_frame_indices=[source_frame_for(i) for i in range(round(TOUR*FPS))],
                background="Full-resolution floating-point field with fine spatial dither",
                long_fictional_data_sentence=False, chapter_numbers=[c[2] for c in CHAPTERS])
    (OUT / "edit-manifest.json").write_text(json.dumps(edit, indent=2), encoding="utf-8")
    try:
        for frame in range(round(TOTAL * FPS)):
            t = frame / FPS
            if t < INTRO:
                image = intro(t)
            elif t < INTRO + TOUR:
                target = source_frame_for(frame - round(INTRO * FPS))
                while decoded_index < target:
                    data = read_exact(reader.stdout, 1600 * 900 * 3)
                    decoded_index += 1
                ui = np.frombuffer(data, np.uint8).reshape(900, 1600, 3)
                image = tour_frame(ui, t - INTRO)
                if t - INTRO < .25:
                    image = Image.blend(intro_card, image, (t - INTRO) / .25)
                last = image.copy()
            else:
                image = outro_card
                if t - INTRO - TOUR < .3:
                    image = Image.blend(last, image, (t - INTRO - TOUR) / .3)
            if t < .35:
                image = Image.blend(BG, image, t / .35)
            if t > TOTAL - .5:
                image = Image.blend(image, BG, (t - TOTAL + .5) / .5)
            writer.stdin.write(image.tobytes())
            if frame % (FPS * 3) == 0:
                image.save(OUT / f"review-{int(t):02d}.png")
                print(f"Edited {t:.0f}/{TOTAL} seconds", flush=True)
        writer.stdin.close()
        assert writer.wait() == 0
        # Drain the unselected tail so the decoder exits normally without a
        # broken pipe. It contains only the same synthetic drive capture.
        while reader.stdout.read(1600 * 900 * 3):
            pass
        reader.stdout.close()
        assert reader.wait() == 0
    finally:
        if writer.poll() is None:
            writer.kill()
        if reader.poll() is None:
            reader.kill()
    finish_audio()


if __name__ == "__main__":
    if "--audio-only" in sys.argv:
        finish_audio()
    else:
        main()
