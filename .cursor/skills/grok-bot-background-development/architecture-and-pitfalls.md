# Grok Bot 架构与故障

Worker 只校验 `D:\grok_bot\Grok Bot\Grok Bot.exe` 完整路径，并从进程命令行读取 9337 调试端口。CDP WebSocket 必须是 127.0.0.1、page target、主 renderer URL；VNC webview 一律跳过。

注入失败时保持官方 Grok Bot 可用，记录错误并允许再次 apply。pause 清理 style、Blob、observer 和计时器；restore 额外清理运行时状态。发布 tag 与 Cargo 版本一致，Release 上传 `GrokBotBackgroundStudio-<version>-plugin.zip`。
