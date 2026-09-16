import { copyFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const assetDirectory = path.dirname(fileURLToPath(import.meta.url));
const pluginDirectory = path.resolve(assetDirectory, '../..');
const iconDirectory = path.join(pluginDirectory, 'src-tauri/icons');
const hostDirectory = process.argv[2] ? path.resolve(process.argv[2]) : null;
const canvasSize = 1024;

const { data: markPixels, info: markInfo } = await sharp(
  path.join(assetDirectory, 'grok-mark-generated.png'),
).ensureAlpha().raw().toBuffer({ resolveWithObject: true });

if (markInfo.width !== canvasSize || markInfo.height !== canvasSize) {
  throw new Error('The generated foreground must be a 1024x1024 image.');
}

// Remove residual glow in nearly transparent pixels and solidify the emblem.
for (let pixelOffset = 0; pixelOffset < markPixels.length; pixelOffset += 4) {
  const alpha = markPixels[pixelOffset + 3];
  const normalizedAlpha = Math.round(Math.max(0, Math.min(1, (alpha - 4) / 236)) * 255);
  markPixels[pixelOffset + 3] = normalizedAlpha;
  if (normalizedAlpha === 0) {
    markPixels.fill(0, pixelOffset, pixelOffset + 3);
  }
}

const foregroundPath = path.join(assetDirectory, 'grok-mark-transparent.png');
await sharp(markPixels, { raw: markInfo }).png().toFile(foregroundPath);

const frameSource = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <rect x="74" y="74" width="876" height="876" rx="208" fill="none" stroke="#20414C" stroke-width="22" stroke-opacity="0.35"/>
  <rect x="74" y="74" width="876" height="876" rx="208" fill="none" stroke="#61D9EC" stroke-width="10" stroke-opacity="0.90"/>
  <rect x="80" y="80" width="864" height="864" rx="202" fill="none" stroke="#C9F4FC" stroke-width="1.5" stroke-opacity="0.40"/>
</svg>`;
await writeFile(path.join(assetDirectory, 'frame.svg'), `${frameSource}\n`);

const composedIcon = await sharp(path.join(assetDirectory, 'background-40.png'))
  .composite([
    { input: Buffer.from(frameSource) },
    { input: foregroundPath },
  ])
  .png()
  .toBuffer();
const masterPath = path.join(pluginDirectory, 'assets/icon.png');
await writeFile(masterPath, composedIcon);

for (const [theme, background] of [['dark', '#161D27'], ['light', '#F2F5F9']]) {
  await sharp(composedIcon).flatten({ background }).resize(512, 512)
    .png().toFile(path.join(assetDirectory, `preview-${theme}.png`));
}

// Refresh the existing Windows icon sizes without touching unused mobile assets.
for (const filename of await readdir(iconDirectory)) {
  if (!filename.endsWith('.png')) continue;
  const outputPath = path.join(iconDirectory, filename);
  const metadata = await sharp(outputPath).metadata();
  await sharp(composedIcon).resize(metadata.width, metadata.height)
    .png().toFile(outputPath);
}

async function createWindowsIcon() {
  const dimensions = [16, 24, 32, 48, 64, 128, 256];
  const images = await Promise.all(dimensions.map((dimension) =>
    sharp(composedIcon).resize(dimension, dimension).png().toBuffer(),
  ));
  // ICO directory entries reference lossless PNG payloads supported by Windows.
  const directory = Buffer.alloc(6 + dimensions.length * 16);
  directory.writeUInt16LE(1, 2);
  directory.writeUInt16LE(dimensions.length, 4);
  let imageOffset = directory.length;
  for (const [imageIndex, dimension] of dimensions.entries()) {
    const entryOffset = 6 + imageIndex * 16;
    directory[entryOffset] = dimension === 256 ? 0 : dimension;
    directory[entryOffset + 1] = dimension === 256 ? 0 : dimension;
    directory.writeUInt16LE(1, entryOffset + 4);
    directory.writeUInt16LE(32, entryOffset + 6);
    directory.writeUInt32LE(images[imageIndex].length, entryOffset + 8);
    directory.writeUInt32LE(imageOffset, entryOffset + 12);
    imageOffset += images[imageIndex].length;
  }
  return Buffer.concat([directory, ...images]);
}

const windowsIcon = await createWindowsIcon();
await writeFile(path.join(iconDirectory, 'icon.ico'), windowsIcon);
await writeFile(path.join(pluginDirectory, 'build/icon.ico'), windowsIcon);

if (hostDirectory) {
  const hostIcon = await sharp(composedIcon).resize(512, 512).png().toBuffer();
  await writeFile(path.join(hostDirectory, 'src-tauri/resources/grok-bot.png'), hostIcon);
  await copyFile(
    path.join(hostDirectory, 'src-tauri/resources/grok-bot.png'),
    path.join(hostDirectory, 'public/plugins/grok-bot.png'),
  );
}

console.log(`Composed ${masterPath}: separate 40% photo, cyan frame and generated foreground.`);
console.log('Exported Windows PNGs and ICO sizes: 16, 24, 32, 48, 64, 128, 256.');
