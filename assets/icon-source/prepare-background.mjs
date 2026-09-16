import { copyFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const assetDirectory = path.dirname(fileURLToPath(import.meta.url));
const sourcePath = path.join(assetDirectory, 'background-source.webp');
const suppliedSourcePath = process.argv[2];

if (suppliedSourcePath && path.resolve(suppliedSourcePath) !== sourcePath) {
  await copyFile(suppliedSourcePath, sourcePath);
}

const canvasSize = 1024;
const tileInset = 64;
const tileSize = canvasSize - tileInset * 2;
const cornerRadius = 220;
const backgroundOpacity = 0.4;

// Keep the photo's alpha separate from the foreground mark and cyan frame.
const roundedMask = Buffer.from(
  `<svg width="${tileSize}" height="${tileSize}">
    <rect width="${tileSize}" height="${tileSize}" rx="${cornerRadius}" fill="white"/>
  </svg>`,
);

const { data: backgroundPixels, info: backgroundInfo } = await sharp(sourcePath)
  .rotate()
  .resize(tileSize, tileSize, { fit: 'cover', position: 'centre' })
  .ensureAlpha()
  .composite([{ input: roundedMask, blend: 'dest-in' }])
  .raw()
  .toBuffer({ resolveWithObject: true });

for (let alphaOffset = 3; alphaOffset < backgroundPixels.length; alphaOffset += 4) {
  backgroundPixels[alphaOffset] = Math.round(
    backgroundPixels[alphaOffset] * backgroundOpacity,
  );
}

await sharp(backgroundPixels, { raw: backgroundInfo })
  .extend({
    top: tileInset,
    bottom: tileInset,
    left: tileInset,
    right: tileInset,
    background: { r: 0, g: 0, b: 0, alpha: 0 },
  })
  .png()
  .toFile(path.join(assetDirectory, 'background-40.png'));

console.log('Prepared background-40.png: 1024x1024, maximum alpha 102/255 (40%).');
