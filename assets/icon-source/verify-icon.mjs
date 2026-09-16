import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const assetDirectory = path.dirname(fileURLToPath(import.meta.url));
const pluginDirectory = path.resolve(assetDirectory, '../..');

async function readPixels(relativePath) {
  return sharp(path.resolve(assetDirectory, relativePath))
    .ensureAlpha().raw().toBuffer({ resolveWithObject: true });
}

function readAlpha(image, column, row) {
  return image.data[(row * image.info.width + column) * 4 + 3];
}

const background = await readPixels('background-40.png');
const foreground = await readPixels('grok-mark-transparent.png');
const composite = await readPixels('../icon.png');
for (const image of [background, foreground, composite]) {
  assert.equal(image.info.width, 1024);
  assert.equal(image.info.height, 1024);
  assert.equal(readAlpha(image, 0, 0), 0, 'Outside corners must be transparent.');
}
let maximumBackgroundAlpha = 0;
for (let alphaOffset = 3; alphaOffset < background.data.length; alphaOffset += 4) {
  maximumBackgroundAlpha = Math.max(maximumBackgroundAlpha, background.data[alphaOffset]);
}
assert.equal(maximumBackgroundAlpha, 102, 'Photo opacity must be exactly 40%.');
assert.equal(readAlpha(foreground, 400, 400), 0, 'The orbital cutout must be transparent.');
assert.equal(readAlpha(composite, 400, 400), 102, 'The cutout must reveal the 40% photo.');
assert.equal(readAlpha(foreground, 512, 512), 255, 'The central slash must be opaque.');
assert.equal(readAlpha(composite, 512, 512), 255);

const windowsIcon = await readFile(path.join(pluginDirectory, 'src-tauri/icons/icon.ico'));
const expectedDimensions = [16, 24, 32, 48, 64, 128, 256];
assert.equal(windowsIcon.readUInt16LE(0), 0);
assert.equal(windowsIcon.readUInt16LE(2), 1);
assert.equal(windowsIcon.readUInt16LE(4), expectedDimensions.length);
for (const [imageIndex, dimension] of expectedDimensions.entries()) {
  const entryOffset = 6 + imageIndex * 16;
  const imageLength = windowsIcon.readUInt32LE(entryOffset + 8);
  const imageOffset = windowsIcon.readUInt32LE(entryOffset + 12);
  assert.ok(imageOffset + imageLength <= windowsIcon.length);
  const metadata = await sharp(windowsIcon.subarray(imageOffset, imageOffset + imageLength))
    .metadata();
  assert.equal(metadata.width, dimension);
  assert.equal(metadata.height, dimension);
  assert.equal(metadata.hasAlpha, true);
}

if (process.argv[2]) {
  const hostDirectory = path.resolve(process.argv[2]);
  const bundledIcon = await readFile(path.join(hostDirectory, 'src-tauri/resources/grok-bot.png'));
  const publicIcon = await readFile(path.join(hostDirectory, 'public/plugins/grok-bot.png'));
  assert.deepEqual(bundledIcon, publicIcon);
  const catalog = JSON.parse(await readFile(
    path.join(hostDirectory, 'src-tauri/resources/catalog.json'), 'utf8',
  ));
  assert.equal(catalog.plugins.find((plugin) => plugin.id === 'grok-bot').icon, 'grok-bot.png');
}

console.log('PASS: 40% photo, transparent cutout/corners, opaque mark, seven ICO sizes and host assets.');
