# NaroLink

NaroLink 在一个工作台里接续 Codex、Claude Code 和 Antigravity 会话。Rust 桌面端运行电脑上的 CLI，Go 服务提供手机网页并转发请求。电脑不需要开放入站端口。两端使用同一套原始会话、模型和运行状态。

## 桌面端

双击 **dist/NaroLink.exe**，从侧栏选择本机会话。模型下拉菜单读取该 CLI 提供的模型。也可以新建会话、选择引擎和电脑上的目录，然后发送消息或图片。

图片支持 PNG、JPEG、GIF、WebP，每次最多 4 张，原图片总计最多 12 MiB。桌面端使用「图片」，网页端使用「图片」或将图片粘贴进输入区。图片经电脑端校验并规范化，随消息交给相应 CLI。最近运行的图片可以在网页中预览。

## 部署 Go 中继

可直接使用 **dist/narolink-relay-linux-amd64**，也可以用 Go 1.23 以上从此目录构建。

~~~bash
go build -trimpath -ldflags="-s -w" -o narolink-relay .
~~~

为服务进程提供以下环境变量。连接密钥应使用至少 32 个字符的高熵随机字符串。

| 变量 | 用途 |
| --- | --- |
| NAROLINK_PUBLIC_URL | 用户在 Safari 打开的 HTTPS 地址，例如 https://narolink.example.com |
| NAROLINK_RELAY_TOKEN | 服务器和电脑端共用的连接密钥 |
| NAROLINK_RELAY_ADDR | 可选，默认 127.0.0.1:7319 |
| NAROLINK_TLS_CERT / NAROLINK_TLS_KEY | 可选，使用 Go 直接提供 HTTPS 时指定证书和私钥文件 |

默认由服务器上的 HTTPS 反向代理转发到 Go 进程，并保留原始 Host。下面是 Caddy 配置示例，使用时换成你的域名。

~~~caddyfile
narolink.example.com {
    reverse_proxy 127.0.0.1:7319
}
~~~

一个中继实例对应一台运行中的电脑。中继只在内存中转发请求，聊天记录由对应 CLI 管理。使用反向代理时，上传请求上限至少设为 36 MiB，响应超时至少设为 45 秒。

## 从 iPhone 连接

1. 在桌面右上角打开 **手机连接**，填入服务器 HTTPS 地址和连接密钥。Windows 可将连接密钥保存在系统凭据中，之后打开 NaroLink 即可重连。
2. 保持电脑和桌面程序运行。用 iPhone 的 Safari 打开网页地址，输入电脑显示的 8 位配对码。
3. 从 Safari **分享 → 添加到主屏幕**，选择 **作为网页 App 打开** 后添加。主屏幕网页若要求重新配对，使用电脑上当前显示的配对码。

桌面也可以从 **手机连接 → 打开本机网页** 进入网页界面。本机端口默认是 4318。无人值守启动可以给桌面进程设置 **NAROLINK_RELAY_URL** 和 **NAROLINK_RELAY_TOKEN**。

[Safari 添加到主屏幕](https://support.apple.com/en-az/guide/iphone/iph42ab2f3a7/ios)

## 源码与验证

~~~powershell
cargo build --release --locked
go build -o dist/narolink-relay.exe .
node scripts/e2e.mjs target/release/narolink.exe dist/narolink-relay.exe
~~~

Windows 若缺少 MSVC 链接器，请在 Visual Studio Developer PowerShell 中构建。可用 **NAROLINK_CODEX_CLI**、**NAROLINK_CLAUDE_CLI**、**NAROLINK_AGY_CLI** 指定 CLI 的绝对路径。

Codex 通过 app-server 读取原会话，在工作区写入沙盒运行。Claude Code 读取本机 JSONL，并以 **--resume** 续聊。Antigravity 通过 ACP 读取和续聊，对来自其桌面存储的会话使用相应的数据目录。工具操作可按 CLI 发出的请求批准一次或拒绝。Codex 的沙盒外权限请求会被拒绝。

CLI 子进程管理和图片校验改编自 Ingenaro MIT 许可 AI 运行时。端到端测试启动独立的 Go 进程和 Rust 进程，用临时会话检查远程配对、图片往返、原 ID 续聊、切换模型以及两端的停止与确认。

### 网页接口

配对后，网页调用以下接口。写请求需要同源 Origin，provider 支持 **codex**、**claude**、**antigravity**。

| 方法与路径 | 结果 |
| --- | --- |
| GET /api/sessions/{provider} | 本机会话索引 |
| GET /api/sessions/{provider}/{id} | 原会话元数据、消息和运行中的任务 |
| GET /api/models/{provider} | 引擎提供的模型 |
| GET /api/activity | 运行中与近期完成的任务 |
| POST /api/runs | 接收 provider、session_id、cwd、model、prompt、images 并返回任务 ID。images 元素含 name 和 Base64 data |
| GET /api/runs/{id} | 回复快照、运行状态和待确认操作 |
| GET /api/runs/{id}/images/{index} | 获取近期任务的规范化 PNG |
| POST /api/runs/{id}/stop | 停止任务 |
| POST /api/runs/{id}/approval | 接收 approval_id 和 allow |
