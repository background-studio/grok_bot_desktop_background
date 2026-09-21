# Grok Bot Background Studio

给 Windows Grok Bot（已在 Electron 42 / 0.57.1 实测）的 Background Studio 协议 2 插件。它通过本机回环 CDP 向 `Grok Bot.exe` 的主 renderer 注入可撤销背景层，不修改 `app.asar`、快捷方式、登录状态或用户数据。0.3.7 起，遇到官方关闭主进程 Inspector 的版本，会临时修改 EXE 的单个 fuse 字节，并在 Grok 完全退出后还原，详见下方安全说明。

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
node --test src/main/native-titlebar.test.cjs src/main/early-transparency.test.cjs
cargo build --release --manifest-path src-tauri/Cargo.toml
```

目标程序默认路径：`D:\grok_bot\Grok Bot\Grok Bot.exe`；调试端口：`9337`。协议和安装包格式见 [docs/plugin-protocol.md](docs/plugin-protocol.md)。

透明标题栏需要首次接管时重启 Grok。启动期间会临时开启回环 Node Inspector，核验进程身份并安装内存补丁后立即关闭。更新插件后请通过宿主重新应用背景，使新启动逻辑生效。

## 临时安全开关与还原（0.3.7）

Grok Bot 0.57.1 关闭了 Electron 的 `EnableNodeCliInspectArguments`，仅添加启动参数不能启用主进程 Inspector。插件保留原生标题栏透明能力，采用受控的临时补丁：

1. 确认目标路径、全部 Grok 进程退出且能独占打开 EXE；备份原始 EXE，写入原版/补丁版 SHA-256 与不可变事务记录。
2. 独立恢复助手就绪后，仅把 Inspector fuse 从 `0` 改成 `1`；其他 fuse、`app.asar` 和用户配置保持不变。
3. 使用回环地址启动 Inspector，验证 PID、程序路径及随机启动标记，安装内存标题栏补丁后立即关闭 Inspector。页面 CDP 仍用于背景同步。
4. Grok 全部进程退出后，助手核对当前 EXE 与备份，仅还原该字节，并验证完整 SHA-256。Windows 锁定运行中的 EXE，因此不能在 Grok 仍运行时恢复磁盘文件。

恢复目录：`%LOCALAPPDATA%\GrokBackgroundStudio\fuse-recovery`。助手按内容哈希复制到此目录，独立于插件安装目录和宿主作业；禁用插件、关闭宿主或 worker 崩溃后仍可继续守护。worker 在启动早期崩溃时，助手也会验证身份并关闭遗留 Inspector。若助手也被结束或机器重启，下次启动插件会继续处理遗留记录；**不是系统服务，不能保证重启后插件未启动时立即恢复**。

临时修改期间，EXE 的 Authenticode 签名会失效；成功还原原始字节后恢复有效。实机验收已确认原始 SHA-256 和签名均恢复。安全软件可能拦截修改，此时插件报错，不降级成不透明标题栏。运行期间不要手动删除备份、记录或助手。官方更新若替换 EXE，助手会拒绝用旧备份覆盖新程序，保留记录并报告错误，需要先检查版本再处理。

`restore` 会停止受管 Grok、还原 EXE 后普通重启；`pause` 不退出 Grok，因此要等 Grok 完全关闭才能还原磁盘开关。仅修改开启原本已关闭的 Inspector 开关，不关闭任何 ASAR 完整性保护。该机制临时放宽了本机调试能力，请只在受信任的本机环境使用。

> 非 Grok Bot 官方产品。Grok Bot 及相关商标归其权利人所有。
