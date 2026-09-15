---
name: grok-bot-background-development
description: Grok Bot Background Studio renderer injection and release workflow.
---

# Grok Bot Background Studio 开发

Grok Bot 0.51.x 是 Electron 应用，主 renderer 是 `file:///D:/grok_bot/Grok%20Bot/resources/app.asar/dist/renderer/index.html`，右侧 Agent Computer 是独立 webview。插件只连接 `127.0.0.1:9337` 的 `page` target，严格拒绝 VNC webview 和公网地址。

## 不可破坏原则

1. 不修改 Grok Bot 安装文件、app.asar、快捷方式、登录状态或用户数据。
2. 只使用精确 renderer URL；注入必须可逆，pause/restore 要清理 style、媒体 Blob、observer 和计时器。
3. CSS 透明度允许 0，媒体层只有一层；不要降低文字和交互控件可读性。
4. 大媒体首帧只注入透明化样式，完整 payload 只 evaluate 一次；修改 CSS 必须改变 payload 修订哈希。
5. 一次性探查文件放在 `poc/`，验收后立即删除。

## 稳定入口

- Bot 列表：`[role="complementary"]` 和 `[aria-label="Bot 列表"]`
- 聊天主区：`main`、`[role="main"]`、`[role="log"]`
- 详情面板：`#sand-conversation-details`
- 对话标题：`#sand-conversation-heading`
- 输入区：`[role="textbox"]`、`[contenteditable="true"]`
- 右侧电脑预览：详情面板内的 `sand-computer-*` 元素；不要把 CSS 注入到其 iframe/webview 内容。

## 生命周期和协议

Rust worker 通过 Named Pipe `\\.\pipe\background-studio-grok-bot` 提供协议 2：`hello`、`configure`、`status`、`apply`、`pause`、`restore`、`shutdown`。目标程序完整路径为 `D:\grok_bot\Grok Bot\Grok Bot.exe`，调试端口为 9337。`shutdown` 保留 Grok Bot 当前窗口。

## 验收

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo build --release --manifest-path src-tauri/Cargo.toml
```

真实验收必须使用运行中的 Grok Bot：configure + apply 后确认 `activeTargets: 1`、`window.__GROK_BACKGROUND_STUDIO__` 为真且 `#grok-background-style` 存在；再执行 pause/restore，确认状态回到 idle 且 activeTargets 为 0。
