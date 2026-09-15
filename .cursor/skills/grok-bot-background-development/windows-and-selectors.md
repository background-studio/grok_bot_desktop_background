# Grok Bot 选择器记录

通过 CDP 无障碍树复核：`#sand-conversation-details` 是详情面板，`#sand-conversation-heading` 是聊天标题，`main` / `[role="main"]` 是聊天主区，`[role="log"]` 是消息记录，`[role="textbox"]` 是输入区。Bot 列表与详情均为 complementary 区域，需按 aria label 或 id 区分。

右侧 Agent Computer 使用独立 webview，renderer worker 只注入主页面，不访问或改写 webview 内容。
