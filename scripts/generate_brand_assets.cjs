// Rasterize the editable Diskvale SVGs and package the same artwork into an ICO.
// Requires a local installation of sharp; see assets/brand/README.md.
const fs = require('node:fs');
const path = require('node:path');
const sharp = require(process.env.DISKVALE_SHARP_MODULE || 'sharp');

function writeIfChanged(filename, bytes) {
  const content = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
  if (fs.existsSync(filename) && fs.readFileSync(filename).equals(content)) return;
  fs.writeFileSync(filename, content);
}

async function main() {
  const root = path.resolve(__dirname, '..');
  const assets = path.join(root, 'assets', 'brand');
  const evidence = path.join(root, 'artifacts', 'diskvale-branding-0.3.22');
  fs.mkdirSync(evidence, { recursive: true });
  const icon = fs.readFileSync(path.join(assets, 'diskvale-icon.svg'));
  const mark = fs.readFileSync(path.join(assets, 'diskvale-mark.svg'), 'utf8');
  const markBody = mark.replace(/^[\s\S]*?<title>.*?<\/title>/, '').replace(/<\/svg>\s*$/, '');
  for (const size of [16, 20, 24, 32, 40, 48, 64, 96, 128, 256, 512]) {
    writeIfChanged(path.join(assets, `diskvale-app-${size}.png`), await sharp(icon, { density: size * 72 / 256 }).resize(size, size).png().toBuffer());
  }
  const frames = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256].map(size => ({
    size, bytes: fs.readFileSync(path.join(assets, `diskvale-app-${size}.png`)),
  }));
  const directory = Buffer.alloc(6 + 16 * frames.length);
  directory.writeUInt16LE(1, 2);
  directory.writeUInt16LE(frames.length, 4);
  let offset = directory.length;
  frames.forEach(({ size, bytes }, index) => {
    const entry = 6 + index * 16;
    directory[entry] = size === 256 ? 0 : size;
    directory[entry + 1] = size === 256 ? 0 : size;
    directory.writeUInt16LE(1, entry + 4);
    directory.writeUInt16LE(32, entry + 6);
    directory.writeUInt32LE(bytes.length, entry + 8);
    directory.writeUInt32LE(offset, entry + 12);
    offset += bytes.length;
  });
  writeIfChanged(path.join(assets, 'diskvale.ico'), Buffer.concat([directory, ...frames.map(frame => frame.bytes)]));
  for (const [filename, ink, body] of [
    ['diskvale-logo.svg', '#EAF4F2', markBody],
    ['diskvale-logo-on-light.svg', '#18333D', markBody.replaceAll('#8BE5D0', '#168875').replaceAll('#AAF0DD', '#227F71').replaceAll('#4EBBDD', '#187DAB')],
  ]) {
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="558" height="180" viewBox="0 0 558 180"><title>Diskvale logo</title><g transform="translate(-18 -12) scale(.8)">${body}</g><text x="196" y="124" font-family="Segoe UI, Ubuntu, sans-serif" font-size="96" font-weight="600" letter-spacing="-3" fill="${ink}">Diskvale</text></svg>`;
    writeIfChanged(path.join(assets, filename), svg + '\n');
    writeIfChanged(path.join(assets, filename.replace('.svg', '.png')), await sharp(Buffer.from(svg)).png().toBuffer());
  }
  const iconBody = icon.toString().replace(/<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '');
  const poster = `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="760" viewBox="0 0 1200 760">
    <defs><symbol id="app" viewBox="0 0 256 256">${iconBody}</symbol><symbol id="mark" viewBox="0 0 256 256">${markBody}</symbol></defs>
    <rect width="1200" height="760" fill="#101B21"/>
    <text x="64" y="74" fill="#7F9DA8" font-family="Segoe UI, sans-serif" font-size="16" letter-spacing="3">DISKVALE / VISUAL IDENTITY</text>
    <use href="#app" x="58" y="126" width="184" height="184"/>
    <text x="282" y="223" fill="#EAF4F2" font-family="Segoe UI, sans-serif" font-size="76" font-weight="600" letter-spacing="-2">Diskvale</text>
    <text x="285" y="269" fill="#8FA7B0" font-family="Segoe UI, sans-serif" font-size="25">Disk space, clearly.</text>
    <line x1="64" y1="360" x2="1136" y2="360" stroke="#2C424C"/>
    <text x="64" y="411" fill="#8FA7B0" font-family="Segoe UI, sans-serif" font-size="17">A storage map, shaped into a D.</text>
    <rect x="64" y="443" width="492" height="235" rx="16" fill="#1B2A32" stroke="#30454F"/>
    <use href="#mark" x="79" y="460" width="176" height="176"/>
    <text x="270" y="554" fill="#EAF4F2" font-family="Segoe UI, sans-serif" font-size="38" font-weight="600">Diskvale</text>
    <text x="270" y="587" fill="#8FA7B0" font-family="Segoe UI, sans-serif" font-size="15">Vector logo · transparent mark</text>
    <rect x="584" y="443" width="552" height="235" rx="16" fill="#EAF0F1"/>
    <use href="#app" x="616" y="472" width="108" height="108"/>
    <text x="750" y="507" fill="#23404C" font-family="Segoe UI, sans-serif" font-size="18" font-weight="600">Windows app icon</text>
    <text x="750" y="536" fill="#5B7480" font-family="Segoe UI, sans-serif" font-size="15">One mark, every size.</text>
    <use href="#app" x="618" y="626" width="16" height="16"/><use href="#app" x="666" y="622" width="24" height="24"/><use href="#app" x="724" y="618" width="32" height="32"/><use href="#app" x="796" y="610" width="48" height="48"/><use href="#app" x="888" y="602" width="64" height="64"/>
    <text x="983" y="643" fill="#5B7480" font-family="Segoe UI, sans-serif" font-size="15">16–256 px ICO</text>
  </svg>`;
  writeIfChanged(path.join(evidence, 'brand-preview.svg'), poster);
  writeIfChanged(path.join(evidence, 'brand-preview.png'), await sharp(Buffer.from(poster)).png().toBuffer());
  process.stdout.write('Generated Diskvale SVG, PNG and 10-size ICO assets.\n');
}
main().catch(error => { process.stderr.write(error.stack + '\n'); process.exitCode = 1; });
