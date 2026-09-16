# Grok Bot icon sources

The icon is assembled locally from separate layers, not generated as a single image.

1. `generation-request.json`: foreground-only prompt for `gpt-image-2.5-flare`.
2. `grok-mark-generated.png`: original image-generation result, with alpha.
3. `grok-mark-transparent.png`: cleaned foreground, with fully transparent cutouts.
4. `background-source.webp`: unchanged copy of the supplied local artwork.
5. `background-40.png`: centered square crop with rounded corners and alpha scaled
   by **0.4** (maximum 102/255). No opaque background is baked into this layer.
6. `frame.svg`: cyan rounded-square border, independent of the photo and mark.

The final 1024px RGBA master is `../icon.png`. Windows PNG sizes and the
multi-resolution ICO are exported to `../../src-tauri/icons/`. The unused legacy
mobile and macOS assets are not part of this Windows plugin's release workflow.

## Rebuild without another API request

From this directory, using Node.js 20.9+:

```powershell
npm ci
npm run prepare-background
npm run compose
npm run verify
```

To also refresh the sibling host's bundled icon:

```powershell
npm run compose -- ../../../background-studio
npm run verify -- ../../../background-studio
```

To replace the source photo, pass its path to `npm run prepare-background --`.
To import a new API response, save it to the ignored `generation-response.json`
and run `npm run import-generation` before composing. API credentials must never
be saved in this directory. The original source image is not sent to the API.

`preview-dark.png` and `preview-light.png` are ignored opaque previews only;
the final exported icons retain their alpha channels.
