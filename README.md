# Grok Bot Background Studio

给 Windows Grok Bot（已在 Electron 42 / 0.53.0 实测）的 Background Studio 协议 2 插件。它通过本机回环 CDP 向 `Grok Bot.exe` 的主 renderer 注入可撤销背景层，不修改 `app.asar`、快捷方式、登录状态或用户数据。

Grok Bot 的右侧 Agent Computer 是独立 `webview`，插件只接管 `file://.../resources/app.asar/dist/renderer/index.html` 页面，因此不会把背景注入到右侧远端 VNC 画面。

## 功能

- 图片 / 视频背景、覆盖 / 适应 / 拉伸 / 平铺、位置、透明度、模糊和遮罩
- 聊天列表、消息区、卡片、菜单分别调节不透明度
- 原生最小化、最大化、关闭按钮区域透出背景，保留官方按钮与主题配色；暂停背景时恢复底色
- 右侧名称、标签、描述输入框跟随“卡片不透明度”，保留文字、边框和聚焦提示
- 新启动 Grok Bot 自动接管，已运行实例可由壳执行“立即接管”
- `pause`、`restore`、`shutdown` 完整清理样式、媒体 Blob、observer 和计时器
- 只连接 `127.0.0.1`，严格校验调试身份、目标类型和 renderer URL

## 开发与验收

要求 Rust stable、Windows MSVC Build Tools；标题栏脚本测试另需 Node.js 24。命令：

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
node --test src/main/native-titlebar.test.cjs
cargo build --release --manifest-path src-tauri/Cargo.toml
```

目标程序默认路径：`D:\grok_bot\Grok Bot\Grok Bot.exe`；调试端口：`9337`。协议和安装包格式见 [docs/plugin-protocol.md](docs/plugin-protocol.md)。

透明标题栏需要首次接管时重启 Grok。启动期间会临时开启回环 Node Inspector，核验进程身份并安装内存补丁后立即关闭；不会修改 Grok 安装文件。更新插件后请通过宿主重新应用背景，使新启动逻辑生效。

> 非 Grok Bot 官方产品。Grok Bot 及相关商标归其权利人所有。
