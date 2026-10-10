"""Edit genuine Diskvale UI footage into a local 1080p product demo."""
import json
import os
import shutil
import math
import subprocess
import sys
import wave
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


def make_audio():
    """Original instrumental: soft plucks, warm chords and understated percussion."""
    rate = 48000
    samples = int(TOTAL * rate)
    stereo = np.zeros((samples, 2), dtype=np.float64)
    rng = np.random.default_rng(20261009)

    def add(time, signal, gain=.1, pan=0):
        start = int(time * rate)
        if start >= samples:
            return
        stop = min(start + len(signal), samples)
        signal = signal[:stop - start] * gain
        stereo[start:stop, 0] += signal * math.sqrt((1 - pan) / 2)
        stereo[start:stop, 1] += signal * math.sqrt((1 + pan) / 2)

    beat = 60 / 108
    chords = [(50, 57, 62, 66), (47, 54, 59, 62), (43, 50, 55, 59), (45, 52, 57, 61)]
    hz = lambda midi: 440 * 2 ** ((midi - 69) / 12)
    for bar in range(math.ceil(TOTAL / (beat * 4))):
        notes = chords[bar % 4]
        t = np.arange(int(beat * 4.4 * rate)) / rate
        env = (1 - np.exp(-t * 7)) * np.exp(-t * .8)
        pad = sum(np.sin(2 * math.pi * hz(n) * t) + .2 * np.sin(2 * math.pi * hz(n) * 2.002 * t) for n in notes) / 4
        add(bar * beat * 4, pad * env, .072, (-.12, .12)[bar % 2])
        for step in range(8):
            t = np.arange(int(.7 * rate)) / rate
            frequency = hz(notes[(step * 3) % 4] + 12)
            pluck = (np.sin(2 * math.pi * frequency * t) + .22 * np.sin(2 * math.pi * frequency * 2 * t))
            pluck *= np.minimum(t * 200, 1) * np.exp(-t * 7)
            add(bar * beat * 4 + step * beat / 2, pluck, .065, (-.35, .35)[step % 2])
    for step in range(math.ceil(TOTAL / beat)):
        t = np.arange(int(.2 * rate)) / rate
        kick = np.sin(2 * math.pi * (48 * t + 48 * .035 * (1 - np.exp(-t / .035)))) * np.exp(-t * 25)
        add(step * beat, kick, .07)
        t = np.arange(int(.07 * rate)) / rate
        noise = rng.normal(size=len(t))
        hat = np.concatenate(([0], np.diff(noise))) * np.exp(-t * 80)
        add(step * beat + beat / 2, hat, .008, .25)
    # Clicks mapped to the trimmed tour, so the score follows the actual input.
    for moment in (INTRO + 4.9, INTRO + 9.1, INTRO + 17.7, INTRO + 22.4):
        t = np.arange(int(.06 * rate)) / rate
        click = np.sin(2 * math.pi * 750 * t) * np.exp(-t * 100)
        add(moment, click, .035)
    time = np.arange(samples) / rate
    stereo *= np.minimum(time / 1.3, 1)[:, None] * np.clip((TOTAL - time) / 1.7, 0, 1)[:, None]
    stereo *= .62 / max(np.abs(stereo).max(), .001)
    with wave.open(str(OUT / "original-score.wav"), "wb") as wav:
        wav.setnchannels(2)
        wav.setsampwidth(2)
        wav.setframerate(rate)
        wav.writeframes((np.clip(stereo, -1, 1) * 32767).astype("<i2").tobytes())


def finish_audio():
    make_audio()
    subprocess.run([FF, "-y", "-v", "error", "-i", str(OUT / "picture.mp4"),
                    "-i", str(OUT / "original-score.wav"), "-map", "0:v:0", "-map", "1:a:0",
                    "-c:v", "copy", "-af", "loudnorm=I=-18:TP=-2:LRA=9", "-c:a", "aac", "-ar", "48000", "-b:a", "192k",
                    "-map_metadata", "-1", "-movflags", "+faststart", "-shortest",
                    str(OUT / EXPORT_NAME)], check=True)
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
