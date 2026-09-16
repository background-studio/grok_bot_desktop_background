(() => {
  const stateKey = "__GROK_BACKGROUND_NATIVE_TITLEBAR__";
  if (globalThis[stateKey]?.installed) return { installed: true };

  const { BrowserWindow } = process.mainModule.require("electron");
  const windows = new Map();
  const rendererProbe = `(() => {
    window.__GROK_BACKGROUND_NATIVE_TITLEBAR__ = true;
    const layer = document.getElementById("grok-background-layer");
    return Boolean(document.documentElement?.classList.contains("grok-background-active")
      && layer && Number(getComputedStyle(layer).opacity) > 0);
  })()`;

  const isMainWindow = (window) => {
    if (window.isDestroyed() || window.webContents.isDestroyed()) return false;
    try {
      const location = new URL(window.webContents.getURL());
      return location.protocol === "file:"
        && decodeURIComponent(location.pathname).replaceAll("\\", "/").toLowerCase()
          .endsWith("/resources/app.asar/dist/renderer/index.html");
    } catch {
      return false;
    }
  };

  const observeWindow = (window) => {
    if (windows.has(window) || typeof window.setTitleBarOverlay !== "function") return;
    const originalSetter = window.setTitleBarOverlay;
    const originalDescriptor = Object.getOwnPropertyDescriptor(window, "setTitleBarOverlay");
    const backgroundColor = window.getBackgroundColor();
    const colorChannels = backgroundColor.slice(-6).match(/.{2}/g)?.map(value => parseInt(value, 16));
    const brightness = colorChannels
      ? (colorChannels[0] * 299 + colorChannels[1] * 587 + colorChannels[2] * 114) / 1000
      : 0;
    const state = {
      active: false,
      pending: false,
      officialOptions: { color: backgroundColor, symbolColor: brightness < 128 ? "#ffffff" : "#000000" },
      originalSetter,
      originalDescriptor,
      wrapper: null,
      onClosed: null,
    };
    state.wrapper = function (options) {
      state.officialOptions = { ...state.officialOptions, ...options };
      return originalSetter.call(this, state.active
        ? { ...options, color: "#00000000" }
        : options);
    };
    state.onClosed = () => windows.delete(window);
    window.setTitleBarOverlay = state.wrapper;
    window.once("closed", state.onClosed);
    windows.set(window, state);
  };

  const restoreWindow = (window, state) => {
    if (window.isDestroyed()) return;
    if (state.active) state.originalSetter.call(window, state.officialOptions);
    if (window.setTitleBarOverlay === state.wrapper) {
      if (state.originalDescriptor) {
        Object.defineProperty(window, "setTitleBarOverlay", state.originalDescriptor);
      } else {
        delete window.setTitleBarOverlay;
      }
    }
    window.removeListener("closed", state.onClosed);
    if (!window.webContents.isDestroyed()) {
      void window.webContents.executeJavaScript(
        "delete window.__GROK_BACKGROUND_NATIVE_TITLEBAR__",
      ).catch(() => {});
    }
  };

  let disposed = false;
  const synchronize = () => {
    for (const window of BrowserWindow.getAllWindows()) {
      if (isMainWindow(window)) observeWindow(window);
    }
    for (const [window, state] of windows) {
      if (!isMainWindow(window)) {
        restoreWindow(window, state);
        windows.delete(window);
        continue;
      }
      if (state.pending) continue;
      state.pending = true;
      // Follow the existing renderer lifecycle, without adding an IPC handler or keeping Inspector open.
      void window.webContents.executeJavaScript(rendererProbe).then(active => {
        if (disposed || window.isDestroyed() || !windows.has(window)) return;
        const nextActive = active === true;
        if (state.active === nextActive) return;
        state.originalSetter.call(window, nextActive
          ? { ...state.officialOptions, color: "#00000000" }
          : state.officialOptions);
        state.active = nextActive;
      }).catch(() => {}).finally(() => { state.pending = false; });
    }
  };
  const timer = setInterval(synchronize, 500);
  timer.unref?.();
  globalThis[stateKey] = {
    installed: true,
    dispose() {
      disposed = true;
      clearInterval(timer);
      for (const [window, state] of windows) restoreWindow(window, state);
      windows.clear();
      delete globalThis[stateKey];
    },
  };
  synchronize();
  return { installed: true };
})()
