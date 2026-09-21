# Background Studio 插件协议

本插件是纯 Rust 无界面 worker，只由 Background Studio 壳启动。

- `pluginProtocol`: `2`
- `pluginId`: `grok-bot`
- 可执行文件：`Grok Bot Background Studio.exe`
- Pipe：`\\.\pipe\background-studio-grok-bot`
- 清单：根级 `plugin.json`
- Release 产物：`GrokBotBackgroundStudio-<version>-plugin.zip`

## 传输

Named Pipe，每行一个 JSON。

请求：

```json
{"id":"...","cmd":"...","params":{}}
```

成功：`{"id":"...","ok":true,"result":{...}}`

失败：`{"id":"...","ok":false,"error":"..."}`

## 命令

| 命令 | 作用 |
|------|------|
| `hello` | 返回 `pluginProtocol:2`、`pluginId`、`version`、`capabilities` |
| `configure` | 校验并下载回环媒体，保存最近一次有效配置；若当前已 active 则热更新 |
| `status` | `phase` / `message` / `activeTargets` / `paused` / `configured` / `revision` |
| `apply` | 使用最近一次有效 configure，手动接管并重新武装 watcher |
| `pause` | 暂停注入并暂停本进程内 watcher |
| `restore` | 恢复官方外观并暂停 watcher |
| `shutdown` | 结束 worker，不改当前 Grok；返回 `shutdown:true`、`keptTarget:true` |

`configure.params`：

```json
{
  "schemaVersion": 1,
  "revision": "...",
  "media": {
    "url": "http://127.0.0.1:<port>/...",
    "kind": "image",
    "mimeType": "image/png",
    "sha256": "<64 hex>",
    "byteSize": 123
  },
  "display": {}
}
```

`hello.capabilities.maxMediaBytes` 与 Manifest 都声明 64 MiB。媒体 URL 仅允许 `http://127.0.0.1` 或 `http://localhost`，拒绝 userinfo、fragment、非回环、端口 0 和超长 JSON。下载使用 `no_proxy`，并校验 `Content-Type`、`Content-Length`、`byteSize`、`sha256`、`mimeType`/`kind`。

未配置时 watcher 只等待并报告「尚未配置背景」，不会杀进程。

## 自动接管

启用后不会自动打开官方 Grok。

- 已有有效调试会话：配置后可重连并注入。
- 配置后用户再普通启动 Grok：按完整 `Grok Bot.exe` 路径确认，关闭该实例，走Grok Bot 调试参数启动路径，再自动应用当前配置。
- 配置前已经在跑的普通进程：不自动关闭；`apply` 才会重启接管。
- `pause` / `restore` 会暂停本插件进程内的 watcher；手动 `apply` 重新武装。
- `shutdown` 或壳停用插件只结束 worker，不改当前 Grok。

## 0.3.7 临时 fuse 事务

- `apply` / 自动接管在启动前建立原版备份、SHA-256 和独立恢复助手，仅临时开启 Inspector fuse，保留完整原生标题栏效果。
- Inspector 在内存补丁安装后立即关闭；`pause` / `shutdown` 不关闭 Grok，所以 EXE 还原由独立助手在全部 Grok 进程退出后完成。
- `restore` 先关闭受管实例，持事务锁还原 EXE，再以官方参数启动。
- 恢复记录位于 `%LOCALAPPDATA%\GrokBackgroundStudio\fuse-recovery`；助手异常退出后，插件下次启动继续恢复。
- 发现版本/hash 不一致时拒绝写入，防止覆盖官方更新。失败状态不会再被 watcher 的等待状态覆盖；恢复错误只作用于对应事务，后续检查成功则解除。
- `integration-test-pipe` 特性仅用于开发验收；其 `GROK_TEST_ABORT_AFTER_SPAWN=1` 故障注入会在启动 Grok 后立即退出测试 worker，验证助手接管。正式包不启用该特性。
