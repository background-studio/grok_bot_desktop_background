import { execFileSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const assetDirectory = path.dirname(fileURLToPath(import.meta.url));
const response = JSON.parse(
  await readFile(path.join(assetDirectory, 'generation-response.json'), 'utf8'),
);
const generatedImage = response.data?.[0];
const outputPath = path.join(assetDirectory, 'grok-mark-generated.png');

if (generatedImage?.b64_json) {
  await writeFile(outputPath, Buffer.from(generatedImage.b64_json, 'base64'));
} else if (generatedImage?.url) {
  const imageUrl = new URL(generatedImage.url);
  if (imageUrl.protocol !== 'https:' && imageUrl.protocol !== 'http:') {
    throw new Error('The image response must use an HTTP(S) download URL.');
  }
  execFileSync('curl.exe', [
    '--fail', '--silent', '--show-error', '--location',
    '--proto', '=http,https', '--proto-redir', '=http,https',
    '--max-time', '120', imageUrl.href, '--output', outputPath,
  ], { stdio: 'inherit' });
} else {
  throw new Error(response.error?.message ?? 'No image found in the generation response.');
}

const metadata = await sharp(outputPath).metadata();
const statistics = await sharp(outputPath).stats();
console.log(JSON.stringify({
  outputPath,
  width: metadata.width,
  height: metadata.height,
  hasAlpha: metadata.hasAlpha,
  alpha: metadata.hasAlpha ? statistics.channels.at(-1) : null,
}, null, 2));
