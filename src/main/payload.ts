import { createHash } from "node:crypto";
import { DisplaySettings, MediaKind } from "../shared/contracts.js";

const BACKGROUND_CSS = String.raw`
html.grok-background-active,
html.grok-background-active body,
html.grok-background-active #root,
html.grok-background-active .bg-app-shell,
html.grok-background-active [data-slot="sidebar-wrapper"],
html.grok-background-active .flex.h-screen.bg-app-shell {
  background: transparent !important;
  background-color: transparent !important;
}

/* 主壳压在媒体层之上，避免内容掉到背景后面。 */
html.grok-background-active body > #root {
  position: relative;
  z-index: 1;
  height: 100%;
  max-height: 100%;
}

#grok-background-layer {
  position: fixed;
  inset: 0;
  z-index: 0;
  overflow: hidden;
  pointer-events: none;
  opacity: calc(var(--cbg-opacity) * var(--cbg-route-intensity));
  background-color: transparent;
  transition: opacity 220ms ease;
}

#grok-background-media,
#grok-background-tile {
  position: absolute;
  left: 0;
  top: 0;
  width: 100%;
  height: 100%;
  transform: scale(var(--cbg-scale));
  filter: blur(var(--cbg-blur));
  transform-origin: center center;
}

#grok-background-media {
  display: block;
  object-fit: var(--cbg-fit);
  object-position: var(--cbg-position-x) var(--cbg-position-y);
}

#grok-background-tile {
  display: none;
  background-image: var(--cbg-media-url);
  background-repeat: repeat;
  background-position: var(--cbg-position-x) var(--cbg-position-y);
  background-size: auto;
}

html.grok-background-fit-tile #grok-background-media { display: none; }
html.grok-background-fit-tile #grok-background-tile { display: block; }

#grok-background-overlay {
  position: absolute;
  inset: 0;
  background: var(--cbg-overlay-color);
  opacity: var(--cbg-overlay-opacity);
}

html.grok-background-home { --cbg-route-intensity: var(--cbg-home-intensity); }
html.grok-background-task { --cbg-route-intensity: var(--cbg-task-intensity); }
html.grok-background-home.grok-background-home-disabled,
html.grok-background-task.grok-background-task-disabled { --cbg-route-intensity: 0; }

/*
 * 侧栏：data-sidebar / sidebar-inner。
 * slider 只加很轻的雾（* 28%），避免高值变成实底罩。
 */
html.grok-background-active [data-sidebar="sidebar"],
html.grok-background-active [data-slot="sidebar-inner"] {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 28%), transparent) !important;
  backdrop-filter: none !important;
  box-shadow: none !important;
}

/* 选中 / 悬停的侧栏入口继续保留文字强调，但底色跟随“左侧边栏”透明度。 */
html.grok-background-active [data-sidebar="menu-button"][data-active],
html.grok-background-active [data-sidebar="menu-button"]:hover,
html.grok-background-active [data-sidebar="menu-button"]:focus-visible {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 100%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 100%), transparent) !important;
  box-shadow: none !important;
}

/* 主画布 / 顶栏 / 卡片壳 / 创建页底栏：外层打底，内部文字层保持可读。 */
html.grok-background-active .bg-page-canvas,
html.grok-background-active header,
html.grok-background-active [data-slot="card"],
html.grok-background-active [data-slot="chat-input-surface"],
html.grok-background-active .pe-chat-launcher {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  backdrop-filter: none !important;
  box-shadow: none !important;
}

/*
 * Create Agent 等路由会在 bg-page-canvas 里再铺一层 bg-background 实底。
 * 外层 canvas 已打雾，内层必须清空，否则整页变黑。
 */
html.grok-background-active .bg-page-canvas .bg-background {
  background: transparent !important;
  background-color: transparent !important;
}

/*
 * 智能体等列表 sticky 表头底部的 ::after / ::before 渐变：
 * 页面透明后会收成一条黑杠（和 Codex sticky 同款问题）。
 */
html.grok-background-active .bg-page-canvas .sticky::before,
html.grok-background-active .bg-page-canvas .sticky::after {
  background: transparent !important;
  background-color: transparent !important;
  background-image: none !important;
  box-shadow: none !important;
  border-color: transparent !important;
}

/*
 * textarea 右下角原生拉伸手柄（::-webkit-resizer）：
 * 创建智能体「指令」等 resize:vertical 框右下角会冒小黑块。
 * 只把 resizer 设成 transparent 不够——Electron 透明窗上这块会透出窗体黑底。
 * 关掉原生 resize，手柄整颗卸掉；scrollbar-corner 一并清掉兜底。
 */
html.grok-background-active textarea {
  resize: none !important;
}
html.grok-background-active textarea::-webkit-resizer {
  background: transparent !important;
  background-color: transparent !important;
  background-image: none !important;
  border: 0 !important;
  box-shadow: none !important;
}
html.grok-background-active textarea::-webkit-scrollbar-corner {
  background: transparent !important;
}

/*
 * 创建页「访问权限」等 radiogroup：选中项原生 bg-muted 是实底灰块，
 * 悬停同款。跟随页面透明度，选中态仍靠 radio 圆点表达。
 */
html.grok-background-active [role="radiogroup"] [role="radio"][class~="bg-muted"],
html.grok-background-active [role="radiogroup"] button[class~="bg-muted"],
html.grok-background-active button[role="radio"][class~="bg-muted"],
html.grok-background-active [role="radiogroup"] [role="radio"]:hover {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 100%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 100%), transparent) !important;
}

/* Electron WCO 把原生窗口按钮叠在同一客户区；顶栏为其预留右侧安全区。 */
html.grok-background-active.grok-background-wco header.relative.shrink-0.h-12 {
  box-sizing: border-box !important;
  padding-right: var(--cbg-wco-safe-right, 0px) !important;
}

/*
 * 看板任务卡片 + 任务列表状态条 + 智能体详情侧卡 + 用量 / 运行时等 shadcn bg-card：
 * 只调整底色，文字与交互保持完整不透明。
 */
html.grok-background-active
  [role="button"][aria-roledescription="sortable"]
  > a[href*="/issues/"]
  > [class~="bg-surface"],
html.grok-background-active
  h3[data-orientation="vertical"][data-index][class~="bg-muted"],
html.grok-background-active
  aside[class~="bg-surface"][class~="border-surface-border"],
html.grok-background-active .bg-card {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-card-opacity) * 100%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-card-opacity) * 100%), transparent) !important;
  box-shadow: none !important;
}

html.grok-background-active [role="dialog"],
html.grok-background-active [role="menu"],
html.grok-background-active [role="listbox"],
html.grok-background-active .bg-surface-raised {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-menu-opacity) * 100%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-menu-opacity) * 100%), transparent) !important;
  backdrop-filter: none !important;
}

/*
 * 附件二级预览（AttachmentPreviewModal）把 role=dialog 打在铺满窗口的遮罩上
 *（fixed inset-0 + 原生 bg-black/80），不是小确认框那张卡。
 * 上面的菜单规则会把整层涂成近实底，壁纸和侧栏都看不见。
 * 遮罩只留点击关闭，不再铺色；中间 max-w-6xl 卡片跟主画布走同一层页面雾，
 * 内层 bg-background / 顶栏 bg-muted/30 / iframe 实底清掉，避免叠两层。
 */
html.grok-background-active [role="dialog"][class~="fixed"][class~="inset-0"] {
  background: transparent !important;
  background-color: transparent !important;
  backdrop-filter: none !important;
}

html.grok-background-active [role="dialog"] [class~="max-w-6xl"] {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  backdrop-filter: none !important;
  box-shadow: none !important;
}

html.grok-background-active [role="dialog"] [class~="max-w-6xl"] .bg-background,
html.grok-background-active [role="dialog"] [class~="max-w-6xl"] iframe,
html.grok-background-active [role="dialog"] [class~="max-w-6xl"] [class~="bg-muted/30"] {
  background: transparent !important;
  background-color: transparent !important;
}

/* Grok Bot 0.51 renderer 的稳定语义入口。sand-* 类名会随构建变化，
 * 这些 role/id 则来自无障碍树和源码常量，分别覆盖 Bot 列表、聊天主区和详情面板。 */
html.grok-background-active [role="complementary"] {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 28%), transparent) !important;
  box-shadow: none !important;
}
html.grok-background-active main,
html.grok-background-active [role="main"],
html.grok-background-active [role="log"] {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  box-shadow: none !important;
}
html.grok-background-active #sand-conversation-details {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  box-shadow: none !important;
}
/* 右侧表单跟随卡片底色；只改背景，保留文字、边框和官方聚焦提示。 */
html.grok-background-active :is(.sand-info-pane, #sand-conversation-details) :is(
  input:not([type]),
  input[type="text"],
  input[type="search"],
  input[type="email"],
  input[type="url"],
  input[type="tel"],
  input[type="password"],
  input[type="number"],
  textarea
) {
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-card-opacity) * 100%), transparent) !important;
}
html.grok-background-active [role="complementary"] button:hover,
html.grok-background-active [role="complementary"] button:focus-visible {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 100%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 100%), transparent) !important;
}
html.grok-background-active [contenteditable="true"],
html.grok-background-active [role="textbox"] {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-composer-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-composer-opacity) * 28%), transparent) !important;
}
/* Grok 0.51 的 sand-shell DOM：这些是 CDP 实测的结构入口，避免依赖会变的哈希类名。 */
html.grok-background-active .sand-shell,
html.grok-background-active .sand-agents-sidebar,
html.grok-background-active .sand-info-pane__inner,
html.grok-background-active .sand-chat,
html.grok-background-active .ui-scroll-area__viewport {
  background: transparent !important;
  background-color: transparent !important;
  box-shadow: none !important;
}
/* Cursor 任务卡片有独立的 article 底色，仅外层 wrapper 透明还不够。 */
html.grok-background-active .sand-widget:is(.sand-widget--choices, .sand-widget--resolved, .sand-widget--dismissed),
html.grok-background-active .sand-cursor-agent-card,
html.grok-background-active .sand-agent-item,
html.grok-background-active .sand-message:not(.sand-activity-line):not(.sand-activity-mark) {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-card-opacity) * 100%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-card-opacity) * 100%), transparent) !important;
  box-shadow: none !important;
}
/* 提问卡片内层选项列表不再叠加实底，仅重设该卡片局部的列表底色变量。
 * 不覆盖按钮样式，保留官方 hover/selected/disabled/focus 和 A/B 快捷键提示。 */
html.grok-background-active .sand-widget:is(.sand-widget--choices, .sand-widget--resolved, .sand-widget--dismissed) {
  --sand-fill-widget-option: transparent;
}
html.grok-background-active .sand-agents-sidebar {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 28%), transparent) !important;
}
html.grok-background-active .sand-agent-item:hover,
html.grok-background-active .sand-agent-item:focus-visible {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 100%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 100%), transparent) !important;
}
/* 运行中三点提示也带 sand-message，但不是消息卡片。
 * line 为紧凑文字提示，mark 为整行图标提示，状态切换时两种都要覆盖。
 * 清空活动行自身底色，直接沿用下方聊天背景的透明度，避免重复叠色；
 * 不修改 opacity 或子元素，保留提示圆点、文字与工作动画。 */
html.grok-background-active .sand-message.sand-activity-line,
html.grok-background-active .sand-message.sand-activity-mark {
  background: transparent !important;
  background-color: transparent !important;
  box-shadow: none !important;
}
html.grok-background-active .sand-chat-stage,
html.grok-background-active .sand-chat-input-dock,
html.grok-background-active .sand-input-area {
  background: transparent !important;
  background-color: transparent !important;
  box-shadow: none !important;
}
html.grok-background-active .sand-chat-input-dock > div,
html.grok-background-active .sand-input-area > div,
html.grok-background-active .sand-input-area > div > div:not(.sand-kit-message-input-frame) {
  background: transparent !important;
  background-color: transparent !important;
  box-shadow: none !important;
}
html.grok-background-active .sand-kit-message-input-frame {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-composer-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-composer-opacity) * 28%), transparent) !important;
  box-shadow: none !important;
}
html.grok-background-active .sand-kit-message-input-frame .sand-prompt-field,
html.grok-background-active .sand-kit-message-input-frame [contenteditable="true"] {
  background: transparent !important;
  background-color: transparent !important;
}
/* 右侧 Agent Computer 的 webview/iframe 保持自身画面，不把聊天背景规则传进去。 */
html.grok-background-active #sand-conversation-details iframe,
html.grok-background-active #sand-conversation-details webview {
  background: transparent !important;
}

html.grok-background-dark #grok-background-layer {
  background-color: transparent;
}

@media (prefers-reduced-motion: reduce) {
  #grok-background-layer { transition: none; }
}
`;

const REVIEW_SHADOW_STYLE_ID = "grok-background-review-shadow-style";
const REVIEW_SHADOW_CSS = String.raw`
/* Grok MVP 暂不注入 Shadow DOM 审阅样式；保留占位以兼容修订哈希与清理逻辑。 */
:host { background-color: transparent !important; }
`;

export interface PayloadInput {
  mediaUrl: string;
  mediaKind: MediaKind;
  display: DisplaySettings;
  revision: string;
}

export function buildRendererPayload(input: PayloadInput) {
  const revision = createHash("sha256")
    .update(input.revision)
    .update(BACKGROUND_CSS)
    .update(REVIEW_SHADOW_CSS)
    .digest("hex");
  const serialized = JSON.stringify({ ...input, revision }).replace(/</g, "\\u003c");
  const css = JSON.stringify(BACKGROUND_CSS);
  const reviewShadowCss = JSON.stringify(REVIEW_SHADOW_CSS);
  const reviewShadowStyleId = JSON.stringify(REVIEW_SHADOW_STYLE_ID);
  return String.raw`(async (config, cssText, reviewShadowCssText, reviewShadowStyleId) => {
    const STATE = "__GROK_BACKGROUND_STUDIO__";
    const STYLE_ID = "grok-background-style";
    const LAYER_ID = "grok-background-layer";
    const REVIEW_HOST_SELECTOR = "diffs-container";
    const ROOT_CLASSES = [
      "grok-background-active", "grok-background-home", "grok-background-task",
      "grok-background-home-disabled", "grok-background-task-disabled",
      "grok-background-fit-tile", "grok-background-dark", "grok-background-wco"
    ];
    const ROOT_PROPERTIES = [
      "--cbg-opacity", "--cbg-blur", "--cbg-scale", "--cbg-fit",
      "--cbg-position-x", "--cbg-position-y", "--cbg-overlay-color",
      "--cbg-overlay-opacity", "--cbg-home-intensity", "--cbg-task-intensity",
      "--cbg-route-intensity", "--cbg-sidebar-opacity", "--cbg-surface-opacity",
      "--cbg-composer-opacity", "--cbg-menu-opacity", "--cbg-terminal-opacity",
      "--cbg-block-fill-opacity", "--cbg-media-url", "--cbg-surface-color",
      "--cbg-wco-safe-right", "--cbg-card-opacity"
    ];

    // New-document scripts can run before the parser has created HTML or BODY.
    if (document.documentElement?.tagName !== "HTML" || !document.body) {
      await new Promise((resolve, reject) => {
        const observer = new MutationObserver(() => {
          if (document.documentElement?.tagName === "HTML" && document.body) {
            observer.disconnect();
            clearTimeout(timeout);
            resolve();
          }
        });
        const timeout = setTimeout(() => {
          observer.disconnect();
          reject(new Error("Grok document was not ready for background installation"));
        }, 15000);
        observer.observe(document, { childList: true, subtree: true });
      });
    }

    // Validate and decode the replacement before removing the visible background.
    const blobUrl = config.mediaUrl;
    const preparedMedia = document.createElement(config.mediaKind === "video" ? "video" : "img");
    preparedMedia.setAttribute("aria-hidden", "true");
    if (config.mediaKind === "video") {
      preparedMedia.autoplay = true;
      preparedMedia.loop = true;
      preparedMedia.muted = Boolean(config.display.videoMuted);
      preparedMedia.defaultMuted = Boolean(config.display.videoMuted);
      preparedMedia.playsInline = true;
      preparedMedia.preload = "auto";
      preparedMedia.playbackRate = Number(config.display.videoPlaybackRate) || 1;
    }
    await new Promise((resolve, reject) => {
      const readyEvent = config.mediaKind === "video" ? "loadeddata" : "load";
      const clearListeners = () => {
        clearTimeout(timeout);
        preparedMedia.removeEventListener(readyEvent, onReady);
        preparedMedia.removeEventListener("error", onError);
      };
      const onReady = () => { clearListeners(); resolve(); };
      const onError = () => {
        clearListeners();
        preparedMedia.removeAttribute("src");
        reject(new Error("Grok background media could not be loaded"));
      };
      const timeout = setTimeout(onError, 15000);
      preparedMedia.addEventListener(readyEvent, onReady, { once: true });
      preparedMedia.addEventListener("error", onError, { once: true });
      preparedMedia.src = blobUrl;
    });
    if (config.mediaKind === "image") {
      // HTMLImageElement.decode() waits for a rendering opportunity and can
      // hang indefinitely in hidden/minimized Electron windows. ImageBitmap
      // performs the same decode validation without waiting for a visible frame.
      const decoded = await createImageBitmap(preparedMedia);
      decoded.close();
    }

    const previous = window[STATE];
    try { previous?.cleanup?.(); } catch { /* 清理旧版本残留后继续安装新版本。 */ }
    {
      if (previous?.observer) previous.observer.disconnect();
      if (previous?.timer) clearInterval(previous.timer);
      previous?.wco?.removeEventListener?.("geometrychange", previous?.wcoGeometry);
      previous?.layer?.remove();
      if (previous?.blobUrl?.startsWith?.("blob:")) URL.revokeObjectURL(previous.blobUrl);
      document.getElementById(LAYER_ID)?.remove();
      document.getElementById(STYLE_ID)?.remove();
    }
    let scheduled = null;
    let shadowPatch = null;

    const installReviewShadowStyle = (host, shadow = host?.shadowRoot) => {
      if (!shadow) return false;
      let shadowStyle = shadow.getElementById(reviewShadowStyleId);
      if (!shadowStyle) {
        shadowStyle = document.createElement("style");
        shadowStyle.id = reviewShadowStyleId;
      }
      if (shadowStyle.dataset.cbgRevision !== config.revision) {
        shadowStyle.textContent = reviewShadowCssText;
        shadowStyle.dataset.cbgRevision = config.revision;
      }
      shadow.appendChild(shadowStyle);
      return true;
    };

    const syncFullWindowMedia = (layer, media, tile) => {
      const viewH = Math.max(Number(window.innerHeight) || 0, 1);
      layer.style.position = "fixed";
      layer.style.inset = "0";
      layer.style.overflow = "hidden";
      layer.style.zIndex = "0";
      for (const node of [media, tile]) {
        if (!node) continue;
        node.style.position = "absolute";
        node.style.left = "0";
        node.style.top = "0";
        node.style.width = "100%";
        node.style.height = viewH + "px";
      }
    };

    const cleanup = () => {
      const state = window[STATE];
      state?.observer?.disconnect();
      if (state?.timer) clearInterval(state.timer);
      state?.wco?.removeEventListener?.("geometrychange", state?.wcoGeometry);
      if (scheduled) cancelAnimationFrame(scheduled);
      if (shadowPatch && shadowPatch.prototype.attachShadow === shadowPatch.wrapped) {
        shadowPatch.prototype.attachShadow = shadowPatch.original;
      }
      document.getElementById(LAYER_ID)?.remove();
      document.getElementById(STYLE_ID)?.remove();
      document.getElementById("grok-background-early-transparency")?.remove();
      document.querySelectorAll("diffs-container").forEach((host) => {
        host.shadowRoot?.getElementById(reviewShadowStyleId)?.remove();
      });
      document.documentElement?.classList.remove(...ROOT_CLASSES);
      for (const property of ROOT_PROPERTIES) document.documentElement?.style.removeProperty(property);
      if (state?.blobUrl?.startsWith?.("blob:")) URL.revokeObjectURL(state.blobUrl);
      delete window[STATE];
      return true;
    };

    const patchAttachShadow = () => {
      const prototype = Element.prototype;
      const original = prototype.attachShadow;
      const wrapped = function(init) {
        const shadow = original.call(this, init);
        if (this.localName === REVIEW_HOST_SELECTOR) {
          queueMicrotask(() => installReviewShadowStyle(this, shadow));
          requestAnimationFrame(() => installReviewShadowStyle(this, shadow));
        }
        return shadow;
      };
      prototype.attachShadow = wrapped;
      return { prototype, original, wrapped };
    };
    // Grok Bot 的右侧电脑预览由独立 webview 承载，不需要接管 Shadow DOM。
    // 不重写 attachShadow，避免影响 React/组件库自己的封装。

    const detectAppearance = () => {
      const root = document.documentElement;
      const classText = ((root?.className || "") + " " + (document.body?.className || ""))
        .toLowerCase()
        .replace(/\bgrok-background-[a-z-]+\b/g, "");
      if (/\b(?:dark|theme-dark)\b/.test(classText)) return "dark";
      if (/\b(?:light|theme-light)\b/.test(classText)) return "light";
      const dataTheme = (
        root?.getAttribute("data-theme") || root?.getAttribute("data-appearance") ||
        document.body?.getAttribute("data-theme") || ""
      ).toLowerCase();
      if (dataTheme.includes("dark")) return "dark";
      if (dataTheme.includes("light")) return "light";
      try {
        return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
      } catch {}
      return "light";
    };

    const install = () => {
      const root = document.documentElement;
      if (!root) return false;

      const setClass = (name, on) => {
        if (root.classList.contains(name) !== on) root.classList.toggle(name, on);
      };
      const setProp = (name, value) => {
        if (root.style.getPropertyValue(name) !== value) root.style.setProperty(name, value);
      };

      const dark = detectAppearance() === "dark";
      setClass("grok-background-dark", dark);
      setProp("--cbg-surface-color", dark ? "#191919" : "#ffffff");
      const wco = navigator.windowControlsOverlay;
      const titlebarRect = wco?.visible ? wco.getTitlebarAreaRect?.() : null;
      const wcoVisible = Boolean(titlebarRect && titlebarRect.width > 0);
      const safeRight = wcoVisible
        ? Math.max(0, window.innerWidth - (titlebarRect.x + titlebarRect.width))
        : 0;
      setClass("grok-background-wco", wcoVisible);
      setProp("--cbg-wco-safe-right", safeRight + "px");

      let style = document.getElementById(STYLE_ID);
      if (!style) {
        style = document.createElement("style");
        style.id = STYLE_ID;
        (document.head || root).appendChild(style);
      }
      if (style.dataset.cbgRevision !== config.revision) {
        style.textContent = cssText;
        style.dataset.cbgRevision = config.revision;
      }

      let layer = document.getElementById(LAYER_ID);
      if (!layer && document.body) {
        layer = document.createElement("div");
        layer.id = LAYER_ID;
        const media = preparedMedia;
        media.id = "grok-background-media";
        media.addEventListener("error", () => {
          if (window[STATE]?.cleanup === cleanup) cleanup();
        }, { once: true });
        const tile = document.createElement("div");
        tile.id = "grok-background-tile";
        const overlay = document.createElement("div");
        overlay.id = "grok-background-overlay";
        layer.append(media, tile, overlay);
        document.body.prepend(layer);
        if (config.mediaKind === "video") media.play().catch(() => undefined);
      }
      if (layer) {
        syncFullWindowMedia(
          layer,
          document.getElementById("grok-background-media"),
          document.getElementById("grok-background-tile"),
        );
      }

      setClass("grok-background-active", true);
      setClass("grok-background-fit-tile", config.display.fit === "tile" && config.mediaKind === "image");
      setClass("grok-background-home-disabled", !config.display.enabledOnHome);
      setClass("grok-background-task-disabled", !config.display.enabledOnTasks);
      setProp("--cbg-opacity", String(config.display.opacity));
      setProp("--cbg-blur", config.display.blur + "px");
      setProp("--cbg-scale", String(config.display.scale));
      setProp("--cbg-fit", config.display.fit === "tile" ? "cover" : config.display.fit);
      setProp("--cbg-position-x", config.display.positionX + "%");
      setProp("--cbg-position-y", config.display.positionY + "%");
      setProp("--cbg-overlay-color", config.display.overlayColor);
      setProp("--cbg-overlay-opacity", String(config.display.overlayOpacity));
      setProp("--cbg-block-fill-opacity", String(config.display.blockFillOpacity));
      setProp("--cbg-home-intensity", String(config.display.homeIntensity));
      setProp("--cbg-task-intensity", String(config.display.taskIntensity));
      setProp("--cbg-sidebar-opacity", String(config.display.sidebarOpacity));
      setProp("--cbg-surface-opacity", String(config.display.surfaceOpacity));
      setProp("--cbg-card-opacity", String(config.display.cardOpacity));
      setProp("--cbg-composer-opacity", String(config.display.composerOpacity));
      setProp("--cbg-menu-opacity", String(config.display.menuOpacity));
      setProp("--cbg-terminal-opacity", String(config.display.terminalOpacity));
      setProp("--cbg-media-url", 'url("' + String(blobUrl).replace(/["\\\n\r]/g, "") + '")');

      // Grok 页面统一按“页面”强度走 home 通道；空白恢复页走 task 通道便于单独关掉。
      const blank = /\/blank(?:\?|$)/.test(location.pathname + location.search);
      setClass("grok-background-home", !blank);
      setClass("grok-background-task", blank);
      return true;
    };

    const scheduleInstall = () => {
      if (scheduled) return;
      scheduled = requestAnimationFrame(() => { scheduled = null; install(); });
    };
    const observer = new MutationObserver(scheduleInstall);
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "data-theme", "data-appearance"],
    });
    const timer = setInterval(install, 4000);
    const wco = navigator.windowControlsOverlay;
    const wcoGeometry = () => scheduleInstall();
    wco?.addEventListener?.("geometrychange", wcoGeometry);
    window[STATE] = {
      revision: config.revision, cleanup, observer, timer, layer: null, blobUrl,
      wco, wcoGeometry,
    };
    install();
    document.getElementById("grok-background-early-transparency")?.remove();
    window[STATE].layer = document.getElementById(LAYER_ID);
    return { installed: true, revision: config.revision, mediaKind: config.mediaKind };
  })(${serialized}, ${css}, ${reviewShadowCss}, ${reviewShadowStyleId})`;
}

export const REMOVE_RENDERER_PAYLOAD = String.raw`(() => {
  const state = window.__GROK_BACKGROUND_STUDIO__;
  if (state?.cleanup) return state.cleanup();
  document.getElementById("grok-background-layer")?.remove();
  document.getElementById("grok-background-style")?.remove();
  document.documentElement?.classList.remove(
    "grok-background-active", "grok-background-home", "grok-background-task",
    "grok-background-home-disabled", "grok-background-task-disabled",
    "grok-background-fit-tile", "grok-background-wco"
  );
  document.documentElement?.style.removeProperty("--cbg-wco-safe-right");
  document.documentElement?.style.removeProperty("--cbg-card-opacity");
  delete window.__GROK_BACKGROUND_STUDIO__;
  return true;
})()`;

export function earlyPayloadFor(payload: string, revision: string) {
  const safeRevision = JSON.stringify(revision);
  return String.raw`(() => {
    const revision = ${safeRevision};
    const run = () => {
      if (document.documentElement?.tagName !== "HTML" || !document.body) return false;
      try { Promise.resolve(${payload}).catch(() => undefined); return true; } catch { return false; }
    };
    if (!run()) {
      const observer = new MutationObserver(() => {
        if (run()) observer.disconnect();
      });
      observer.observe(document.documentElement || document, { childList: true, subtree: true });
      setTimeout(() => observer.disconnect(), 30000);
    }
    return revision;
  })()`;
}
