// Convert the existing notebook SVG to a multi-resolution Windows icon.
// Requires sharp: node app-icon.cjs [absolute path to sharp module]
const fs = require('node:fs');
const path = require('node:path');
const sharp = require(process.argv[2] || 'sharp');

async function main() {
  const source = path.join(__dirname, '../skin/notebook.svg');
  const out = path.join(__dirname, '../icons');
  fs.mkdirSync(out, { recursive: true });
  const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
  const frames = await Promise.all(sizes.map(size =>
    sharp(source, { density: 384 }).resize(size, size).png().toBuffer()));
  const header = Buffer.alloc(6 + sizes.length * 16);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  frames.forEach((frame, i) => {
    const pos = 6 + i * 16;
    header[pos] = header[pos + 1] = sizes[i] === 256 ? 0 : sizes[i];
    header.writeUInt16LE(1, pos + 4);
    header.writeUInt16LE(32, pos + 6);
    header.writeUInt32LE(frame.length, pos + 8);
    header.writeUInt32LE(offset, pos + 12);
    offset += frame.length;
  });
  fs.writeFileSync(path.join(out, 'zijian.ico'), Buffer.concat([header, ...frames]));
  fs.writeFileSync(path.join(out, 'zijian.png'), frames[frames.length - 1]);
}
main().catch(error => { console.error(error); process.exitCode = 1; });
