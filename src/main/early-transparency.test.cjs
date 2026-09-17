const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { test } = require("node:test");
const { runInNewContext } = require("node:vm");

const injectorSource = readFileSync(join(__dirname, "../../src-tauri/src/injector.rs"), "utf8");
const earlyScript = injectorSource.match(/const EARLY_TRANSPARENCY_SCRIPT: &str = r#"([\s\S]*?)"#;/)?.[1];
assert.ok(earlyScript, "Read the exact early script embedded in the Rust worker");

function createDocumentFixture(initialRoot = null) {
  const styles = [];
  let mutationCallback;
  let disconnected = false;
  let documentAppends = 0;
  const document = {
    documentElement: initialRoot,
    createElement: tagName => ({ tagName: tagName.toUpperCase() }),
    getElementById: identifier => styles.find(style => style.id === identifier),
    appendChild() { documentAppends += 1; },
  };
  const context = {
    document,
    MutationObserver: class {
      constructor(callback) { mutationCallback = callback; }
      observe(target) { assert.equal(target, document); }
      disconnect() { disconnected = true; }
    },
    setTimeout() {},
  };
  const htmlRoot = { tagName: "HTML", appendChild: style => styles.push(style) };
  return {
    document, htmlRoot, styles,
    run: () => runInNewContext(earlyScript, context),
    notifyParserMutation: () => mutationCallback?.(),
    disconnected: () => disconnected,
    documentAppends: () => documentAppends,
  };
}

test("early transparency waits for HTML instead of creating a STYLE document root", () => {
  for (const initialRoot of [null, { tagName: "STYLE" }]) {
    const fixture = createDocumentFixture(initialRoot);
    fixture.run();
    assert.equal(fixture.documentAppends(), 0);
    assert.equal(fixture.styles.length, 0);
    fixture.document.documentElement = fixture.htmlRoot;
    fixture.notifyParserMutation();
    assert.equal(fixture.styles.length, 1);
    assert.equal(fixture.styles[0].id, "grok-background-early-transparency");
    assert.equal(fixture.document.documentElement, fixture.htmlRoot);
    assert.equal(fixture.disconnected(), true);
  }
});

test("early transparency is idempotent when the parser already created HTML", () => {
  const fixture = createDocumentFixture();
  fixture.document.documentElement = fixture.htmlRoot;
  fixture.run();
  fixture.run();
  assert.equal(fixture.styles.length, 1);
  assert.equal(fixture.documentAppends(), 0);
  assert.ok(fixture.styles[0].textContent.includes("background:transparent"));
});
