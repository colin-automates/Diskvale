# Demo audio

**Sirens in Darkness** by **The Cynic Project** ([cynicmusic.com](https://cynicmusic.com/) / [pixelsphere.org](https://pixelsphere.org/)), from the 2017 Pixelsphere soundtrack.

The artist publishes this recording under [CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/) on the [original upload page](https://opengameart.org/content/sirens-in-darkness). The recording and included excerpt retain that dedication; Diskvale's MIT license does not replace it. This credit also honors the artist's request for attribution. No endorsement by the artist is implied.

`sirens-in-darkness-excerpt.mp3` contains approximately seconds 12–42 of the recording, trimmed by copying MP3 packets without re-encoding. `soundtrack.json` records the download URL, original and excerpt SHA-256 hashes, and trim details. The demo renderer adds 1.3-second opening and 2.5-second ending fades and normalizes the music alone to -26 LUFS, 6 dB below the previous piano-only mix.

**Middle Mouse Click** by **1j01** is a [recording of a physical mouse button](https://opengameart.org/content/middle-mouse-click), published by its recorder under [CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/). `middle-click-press.wav` and `middle-click-release.wav` are unmodified downloads. `clicks.json` records their source URLs, hashes, and the four click cues retained from the capture script. The mix applies a 120 Hz high-pass and 5 kHz low-pass filter, scales presses to -18 dBFS and releases to -22 dBFS peak, and aligns them with the capture's press/release frames. It adds no synthesized click tones. The music level stays fixed when these sounds are mixed in.

The finished demo remains 30 seconds long. The public MP4 is `docs/media/diskvale-demo.mp4`; the [rendering instructions](../../tools/demo-video/README.md) describe how to reproduce it.
