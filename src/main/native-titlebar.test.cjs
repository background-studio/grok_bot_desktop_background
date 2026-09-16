const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { test } = require("node:test");
const { EventEmitter } = require("node:events");
const { runInNewContext } = require("node:vm");

const patchSource = readFileSync(join(__dirname, "native-titlebar.cjs"), "utf8");

function createFixture() {
  const overlayCalls = [];
  const window = new EventEmitter();
  let backgroundActive = false;
  let destroyed = false;
  let synchronize;
  let timerCleared = false;
  let rendererUrl = "file:///D:/grok_bot/Grok%20Bot/resources/app.asar/dist/renderer/index.html";
  const originalSetter = function (options) { overlayCalls.push({ ...options }); };
  Object.assign(window, {
    isDestroyed: () => destroyed,
    getBackgroundColor: () => "#191919",
    setTitleBarOverlay: originalSetter,
    webContents: {
      isDestroyed: () => destroyed,
      getURL: () => rendererUrl,
      executeJavaScript: async () => backgroundActive,
    },
  });
  const context = {
    URL,
    process: { mainModule: { require: () => ({ BrowserWindow: { getAllWindows: () => [window] } }) } },
    setInterval(callback) { synchronize = callback; return { unref() {} }; },
    clearInterval() { timerCleared = true; },
  };
  runInNewContext(patchSource, context);
  return {
    window,
    context,
    overlayCalls,
    originalSetter,
    setBackgroundActive(value) { backgroundActive = value; },
    setRendererUrl(value) { rendererUrl = value; },
    destroyWindow() { destroyed = true; window.emit("closed"); },
    timerCleared: () => timerCleared,
    async synchronize() {
      await new Promise(resolve => setImmediate(resolve));
      synchronize();
      await new Promise(resolve => setImmediate(resolve));
    },
  };
}

test("native transparency preserves theme symbols and restores latest official options on pause", async () => {
  const fixture = createFixture();
  fixture.setBackgroundActive(true);
  await fixture.synchronize();
  assert.equal(fixture.overlayCalls.at(-1).color, "#00000000");
  fixture.window.setTitleBarOverlay({ color: "#fafafa", symbolColor: "#000000", height: 52 });
  assert.deepEqual(fixture.overlayCalls.at(-1), { color: "#00000000", symbolColor: "#000000", height: 52 });
  fixture.setBackgroundActive(false);
  await fixture.synchronize();
  assert.deepEqual(fixture.overlayCalls.at(-1), { color: "#fafafa", symbolColor: "#000000", height: 52 });
});

test("disposal restores the setter, official color, listeners and timer", async () => {
  const fixture = createFixture();
  fixture.setBackgroundActive(true);
  await fixture.synchronize();
  fixture.context.__GROK_BACKGROUND_NATIVE_TITLEBAR__.dispose();
  assert.equal(fixture.window.setTitleBarOverlay, fixture.originalSetter);
  assert.equal(fixture.overlayCalls.at(-1).color, "#191919");
  assert.equal(fixture.window.listenerCount("closed"), 0);
  assert.equal(fixture.context.__GROK_BACKGROUND_NATIVE_TITLEBAR__, undefined);
  assert.equal(fixture.timerCleared(), true);
});

test("navigation away from the app restores native appearance and stops touching the window", async () => {
  const fixture = createFixture();
  fixture.setBackgroundActive(true);
  await fixture.synchronize();
  fixture.setRendererUrl("https://example.com/");
  await fixture.synchronize();
  assert.equal(fixture.window.setTitleBarOverlay, fixture.originalSetter);
  assert.equal(fixture.overlayCalls.at(-1).color, "#191919");
  const callCount = fixture.overlayCalls.length;
  fixture.destroyWindow();
  await fixture.synchronize();
  assert.equal(fixture.overlayCalls.length, callCount);
});
