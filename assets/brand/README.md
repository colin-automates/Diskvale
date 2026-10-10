# Diskvale identity

The mark is a segmented D: a vertical storage tile and two curved tiles around a clear center. Mint and cyan connect it to the disk map's palette. The app icon places the mark on a dark rounded square with a subtle border.

- `diskvale-icon.svg`: editable app-icon master.
- `diskvale-mark.svg`: flat mark with a transparent background.
- `diskvale-logo.svg` / `.png`: horizontal logo for dark backgrounds.
- `diskvale-logo-on-light.svg` / `.png`: horizontal logo for light backgrounds.
- `diskvale-app-{size}.png`: transparent app icons at 16, 20, 24, 32, 40, 48, 64, 96, 128, 256 and 512 pixels.
- `diskvale.ico`: Windows icon containing ten sizes from 16 through 256 pixels.

SVG wordmarks use editable Segoe UI text with Ubuntu/sans-serif fallbacks. The PNG wordmarks preserve the rendered typography. Keep the mark's proportions and the gaps between its tiles.

Regenerate the PNGs and ICO from the SVG masters with `node scripts/generate_brand_assets.cjs`. Install the optional `sharp` module with `npm install --no-save --package-lock=false sharp` first, or supply its module location through `DISKVALE_SHARP_MODULE`. It does not rewrite unchanged assets. These tools are not needed for an ordinary Rust build.

The UI embeds the 64-pixel header artwork and 256-pixel native-window artwork. The Windows build embeds the ICO and version information through the Windows SDK resource compiler. The portable executable includes the artwork and needs no external image files.
