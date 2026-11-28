window.NARO_I18N = {
  "zh-CN": {
    pageTitle: "Naro — 跨设备无痕接力你的本地 AI 编程会话",
    navWorkspace: "工作台",
    navRelay: "中继架构",
    navFeatures: "核心能力",
    navDeploy: "自托管部署",
    navDownload: "下载 Naro",
    eyebrow: "支持 Codex · Claude Code · Antigravity",
    heroLine1: "会话留在你的电脑，",
    heroLine2Prefix: "随时随地",
    heroLine2Accent: "无痕接力",
    heroSubtitle:
      "在目标电脑运行原生 CLI，在另一台电脑的 Naro 桌面端，或手机与平板的浏览器中无缝续聊、审批、看 Diff 与推送代码。Go 中继仅内存转发，零数据落盘。",
    ctaDownload: "下载桌面端 (macOS / Windows)",
    ctaDeploy: "部署 Go 中继",
    metaRust: "Rust 原生桌面端",
    metaGo: "Go 零落盘单 WSS 中继",
    metaMit: "MIT 开源 · 无需开放端口",

    stageWss: "1 条 WSS 长连接 · 零落盘",
    tabChat: "会话流",
    tabGit: "Git 管理",
    tabUsage: "用量统计",
    sidebarProjects: "工作区项目",
    sidebarQuotaLabel: "24h Token 用量",
    sidebarQuotaSub: "含缓存读取与写入",
    phoneConnected: "已连 Mac Studio",
    phoneApprovalTitle: "待审批命令",
    phoneApproveBtn: "批准执行",
    phoneSteerBtn: "引导当前轮",
    phoneToastIdle: "WSS 隧道在线 · 延迟 18ms",
    phoneToastApproved: "✓ 已回传批准指令 (req_id #a41f)",
    phoneToastSteered: "✓ 已通过 Steer 送入当前轮",
    desktopSteerNotice: "来自 iPhone · 引导会话：顺便补上断线重连的单元测试",
    desktopApprovedNotice: "来自 iPhone · 已批准命令执行，继续流式输出",
    composerPlaceholder: "Enter 发送 · Shift+Enter 换行 · ⌘V 粘贴截图",
    composerMode: "队列 / 引导",

    gitActionStage: "暂存更改",
    gitActionCommit: "提交 (含 Amend)",
    gitActionPush: "Push 远端",
    usage24h: "最近 24 小时",
    usage7d: "最近 7 天",
    usage30d: "最近 30 天",

    enginesKicker: "NATIVE CLI ENGINES",
    enginesTitle: "三大编码引擎，原生协议直连",
    enginesDesc:
      "Naro 不发明封闭格式，也不接管你的对话数据。所有会话历史直接读写目标电脑上原生 CLI 的状态，随时可回到终端继续。",
    codexDesc:
      "通过 app-server 读取并接续原会话，按登记项目自动归组；支持推理强度、闪电快速模式、四档权限预设以及账号额度一键重置。",
    codexTags: ["app-server", "项目归组", "快速模式", "额度重置"],
    claudeDesc:
      "直读本机 JSONL 会话历史并以 --resume 原 ID 续聊；完整映射 CLI 权限模式与套餐额度查询，保持与本机终端一致的上下文。",
    claudeTags: ["本机 JSONL", "--resume 续聊", "原生权限模式", "套餐额度"],
    agyDesc:
      "通过 ACP 协议读取并接续 Antigravity 会话，兼容 Antigravity 桌面端数据目录与 SQLite 历史统计，支持切换 CLI 执行模式。",
    agyTags: ["ACP 协议", "SQLite 历史", "CLI 执行模式", "桌面端目录"],

    relayKicker: "ZERO-PERSISTENCE ARCHITECTURE",
    relayTitle: "不落盘的 Go 中继，一条 WSS 并发复用",
    relayDesc:
      "目标电脑只需主动发起 1 条出站 WSS 长连接，无需公网 IP 或防火墙端口映射。聊天记录、附件、文件预览与配对记录从不写入中继磁盘。",
    nodeClientTag: "ACCESS CLIENT",
    nodeClientTitle: "访问设备",
    nodeClientDesc: "手机 / 平板浏览器（PWA）或另一台电脑的 Naro 桌面端，使用标准 HTTPS API 与 SSE。",
    nodeRelayTag: "IN-MEMORY ONLY",
    nodeRelayTitle: "Naro Go 中继",
    nodeRelayDesc: "仅在内存维护设备路由与在线状态，按请求 ID 复用单条 WSS，结束或断线即清理。",
    nodeHostTag: "TARGET HOST",
    nodeHostTitle: "目标电脑",
    nodeHostDesc: "运行 Naro 桌面端与本机 CLI，验证配对码、读取项目文件并管理完整会话历史。",
    pipeHttps: "HTTPS / SSE",
    pipeHttpsSub: "同源校验",
    pipeWss: "单条 WSS",
    pipeWssSub: "主动出站",

    simStream: "发送追问 (SSE 流式)",
    simApproval: "流式中并发审批",
    simStop: "即时停止生成",
    simHeartbeat: "空闲 25s 心跳",
    telDisk: "中继落盘",
    telPorts: "需开放入站端口",
    telWss: "目标机连接数",
    simLogDefault: "点击上方场景，观察请求如何经单条 WSS 在内存中零落盘转发。",
    simLogStream:
      "[req_id=7c02] POST /api/runs → Go 中继内存复用 WSS → 目标机启动 CLI → SSE 增量回传（0 B 落盘）",
    simLogApproval:
      "[req_id=7c09] 流式回复尚未结束，审批指令经同一 WSS 并发直达目标机，无需等待当前轮阻塞",
    simLogStop:
      "[req_id=7c14] POST /api/runs/{id}/stop → 目标机立即终止任务，队列中剩余消息标记为未发送",
    simLogHeartbeat:
      "[keep-alive] 空闲时段仅每 25 秒收发极轻量心跳；断线后带随机抖动退避重连，绝不重复写操作",
    honestTitle: "公开透明：",
    honestText:
      "Naro Go 中继本身不持久化任何用户数据；官方部署的 Nginx 反向代理会记录包含来源 IP 与请求路径的标准访问日志。你随时可以用一条命令部署自己的私有中继。",

    featuresKicker: "BUILT FOR REAL WORKFLOWS",
    featuresTitle: "不只是手机壳，是完整的远程编码工作台",
    featuresDesc: "把在电脑前顺手的每一个细节，都带到沙发、通勤路上与另一台设备。",

    featQueueTitle: "队列、引导与撤回修改",
    featQueueDesc:
      "回复期间随时发送后续消息：默认「加入队列」按序执行，或用「引导会话 (Steer)」直接注入当前轮；支持撤回排队消息编辑并通过「切换草稿」找回原内容。",
    featQueueRow1: "补全 WebSocket 断线重试测试",
    featQueueBadge1: "引导当前轮",
    featQueueRow2: "顺便更新 README.en.md 中的 Nginx 配置",
    featQueueBadge2: "撤回修改",

    featGitTitle: "内置远程 Git 管理",
    featGitDesc:
      "桌面端与网页端均内置 Git 面板，实时跟随当前工作目录：查看状态与文件 Diff、暂存/取消暂存、丢弃更改、提交（含 Amend）、切换分支及 Fetch / Pull / Push。",

    featUsageTitle: "Token 用量与账号额度",
    featUsageDesc:
      "汇总目标电脑最近 24 小时、7 天与 30 天的总消耗及分引擎明细（含缓存读写），随时查看各引擎剩余额度与重置时间，Codex 账号可确认使用重置次数。",

    featPreviewTitle: "只读文件预览与图片粘贴",
    featPreviewDesc:
      "点击回复中的文件链接即可在侧栏或全屏预览（Markdown 渲染/原文切换，上限 2 MiB）；支持 ⌘V / Ctrl+V 粘贴最多 4 张、合计 12 MiB 的截图与图片。",

    featPairTitle: "扫码配对与多设备切换",
    featPairDesc:
      "目标电脑生成一次性二维码与 8 位配对码，手机扫码即连并支持 Safari 添加到主屏幕；同一中继可接入多台电脑，在桌面端或网页侧栏一键切换。",
    pairCodeLabel: "8 位本机配对码",
    pairCodeSub: "由目标电脑本地验证，中继不存储凭证",

    featCraftTitle: "为工程师打磨的原生细节",
    featCraftDesc:
      "Rust (eframe/egui) 构建轻量桌面端，零框架原生 Web 工作台；支持简体中文 / English / 日本語、浅色/深色/跟随系统同步，以及独立的 CLI HTTP/HTTPS 代理配置。",
    craftPills: [
      "简体中文 · EN · 日本語",
      "浅色 / 深色 / 跟随系统",
      "CLI 独立代理",
      "macOS & Windows",
      "局域网本机网页 127.0.0.1:4318",
      "无框架极速前端"
    ],

    stepsKicker: "THREE MINUTES TO FLOW",
    stepsTitle: "三步，把你的主力开发机连到口袋里",
    step1Title: "在目标电脑打开 Naro",
    step1Desc: "安装并登录 codex、claude 或 agy，解压运行 Naro.app（macOS）或 Naro.exe（Windows）。",
    step2Title: "配置中继地址与密钥",
    step2Desc: "在「设备连接 → 本机配对」填入中继 HTTPS 地址与连接密钥，点击连接即建立单条出站 WSS。",
    step3Title: "扫码或输码，随时接力",
    step3Desc: "手机打开网页扫码，或在另一台电脑的 Naro 中输入设备地址与 8 位配对码，立即续写工作流。",

    deployKicker: "SELF-HOST IN SECONDS",
    deployTitle: "单文件 Go 中继，两分钟私有部署",
    deployDesc: "无需数据库，无需 Redis。可直接运行单个 Go 二进制，或通过 Docker / Docker Compose 一键容器化部署。",
    codeTabGo: "Go 中继",
    codeTabDocker: "Docker",
    codeTabBuild: "从源码构建",
    copyLabel: "复制代码",
    copiedLabel: "已复制",
    envPublicDesc: "中继公开 HTTPS 地址（协议 + 主机名 + 可选端口）",
    envTokenDesc: "中继与目标电脑共享的 32–512 位无空格随机密钥",
    envAddrDesc: "中继监听地址，默认 127.0.0.1:7319",
    envTlsDesc: "可选；由 Go 中继直接提供 HTTPS 时指定证书与私钥",

    closingTitle: "离开书桌，不必打断思路。",
    closingDesc: "Naro 以 MIT 协议开源，支持 macOS 与 Windows。下载安装包，或直接编译运行你的私有中继。",
    closingPrimary: "前往 GitHub Releases 下载",
    closingSecondary: "查看开源仓库",
    footerMeta: "MIT 许可证 · 随附字体 Noto Sans CJK SC (SIL OFL 1.1)",
    footerZh: "简体中文 README",
    footerEn: "English README",
    footerJa: "日本語 README",

    engines: {
      codex: {
        sessionTitle: "优化 Go 中继 WSS 心跳与重连退避",
        cwd: "~/WorkSpace/naro-link",
        model: "gpt-5.4 · High",
        fast: true,
        perm: "请求批准",
        userMsg: "帮我检查 relay_ws.go 里的断线重连逻辑，确保不会重复执行写请求。",
        assistantHtml:
          '已定位 <span class="file-chip">relay_ws.go</span> 中的重连循环。建议在退避窗口引入随机抖动，并在连接断开时立即清理挂起的 <code>pendingRequests</code>，不自动重放非幂等请求。',
        phoneCmd: "go test ./... -race -count=1",
        phoneSummary: "Codex 已提出补丁，准备运行 Go 竞态检测验证重连状态机。"
      },
      claude: {
        sessionTitle: "重构桌面端侧栏项目分组与文件预览",
        cwd: "~/WorkSpace/naro-link",
        model: "claude-sonnet-4-6",
        fast: false,
        perm: "CLI 权限模式",
        userMsg: "把超过 256 KiB 的 Markdown 文件降级为纯源码预览，避免大文档卡顿。",
        assistantHtml:
          '已在 <span class="file-chip">src/file_preview.rs</span> 与 <span class="file-chip">public/app.js</span> 同步加入 256 KiB 阈值判断，保留 2 MiB 单文件读取上限与一键复制。',
        phoneCmd: "cargo test --locked file_preview",
        phoneSummary: "Claude Code 完成大文件预览降级，请求运行单元测试。"
      },
      antigravity: {
        sessionTitle: "梳理移动端 PWA 主题同步与安全区适配",
        cwd: "~/WorkSpace/naro-link",
        model: "gemini-3.5-pro",
        fast: true,
        perm: "ACP 执行模式",
        userMsg: "排查 iOS Safari 添加到主屏幕后的底部安全区间距和标签页主题同步。",
        assistantHtml:
          '已检查 <span class="file-chip">public/style.css</span> 的 <code>env(safe-area-inset-bottom)</code> 与 <span class="file-chip">public/theme.js</span> 的 <code>storage</code> 监听，同浏览器多标签页可实时同步。',
        phoneCmd: "node scripts/theme-e2e.mjs",
        phoneSummary: "Antigravity 已更新安全区样式，准备执行主题端到端测试。"
      }
    }
  },

  en: {
    pageTitle: "Naro — Seamless Handoff for Your Local Coding Agents",
    navWorkspace: "Workspace",
    navRelay: "Relay",
    navFeatures: "Features",
    navDeploy: "Self-Host",
    navDownload: "Download",
    eyebrow: "Works with Codex · Claude Code · Antigravity",
    heroLine1: "Your agents stay home.",
    heroLine2Prefix: "Your flow goes ",
    heroLine2Accent: "everywhere.",
    heroSubtitle:
      "Naro runs native CLIs on your host computer so you can pick up sessions, approve actions, review Git diffs, and steer runs from another desktop, phone, or tablet—over a zero-persistence Go relay.",
    ctaDownload: "Download Desktop (macOS / Windows)",
    ctaDeploy: "Deploy Go Relay",
    metaRust: "Native Rust Desktop App",
    metaGo: "Stateless Single-WSS Go Relay",
    metaMit: "MIT Licensed · Zero Inbound Ports",

    stageWss: "1 Outbound WSS · Zero Disk",
    tabChat: "Session",
    tabGit: "Git",
    tabUsage: "Usage",
    sidebarProjects: "Workspaces",
    sidebarQuotaLabel: "24h Token Usage",
    sidebarQuotaSub: "Includes cache read/write",
    phoneConnected: "Linked · Mac Studio",
    phoneApprovalTitle: "Approval Required",
    phoneApproveBtn: "Approve",
    phoneSteerBtn: "Steer Turn",
    phoneToastIdle: "WSS online · 18ms RTT",
    phoneToastApproved: "✓ Approval sent (req_id #a41f)",
    phoneToastSteered: "✓ Steered into active turn",
    desktopSteerNotice: "From iPhone · Steer: also add reconnect unit tests while you're there",
    desktopApprovedNotice: "From iPhone · Command approved, continuing live stream",
    composerPlaceholder: "Enter to send · Shift+Enter for newline · ⌘V paste screenshot",
    composerMode: "Queue / Steer",

    gitActionStage: "Stage",
    gitActionCommit: "Commit (Amend)",
    gitActionPush: "Push",
    usage24h: "Last 24h",
    usage7d: "Last 7 days",
    usage30d: "Last 30 days",

    enginesKicker: "NATIVE CLI ENGINES",
    enginesTitle: "Three coding agents, true native fidelity",
    enginesDesc:
      "Naro never traps your history in a proprietary format. Each CLI manages its own sessions on your host, so desktop, mobile, and terminal stay completely in sync.",
    codexDesc:
      "Connects via app-server with project-grouped sessions, reasoning effort controls, lightning fast-mode toggle, four permission presets, and account quota reset.",
    codexTags: ["app-server", "Project groups", "Fast mode", "Quota reset"],
    claudeDesc:
      "Reads native local JSONL session history and continues with --resume. Surfaces CLI permission modes and account allowances without breaking local state.",
    claudeTags: ["Local JSONL", "--resume", "CLI permissions", "Plan limits"],
    agyDesc:
      "Connects over ACP with support for Antigravity desktop data directories, SQLite session usage analytics, and native CLI execution modes.",
    agyTags: ["ACP protocol", "SQLite usage", "Execution modes", "Desktop data"],

    relayKicker: "ZERO-PERSISTENCE ARCHITECTURE",
    relayTitle: "A stateless Go relay, multiplexed over one WSS",
    relayDesc:
      "Your host computer opens a single outbound WSS connection—no inbound ports or firewall holes required. Messages, images, file previews, and pairing records never touch the relay's disk.",
    nodeClientTag: "ACCESS CLIENT",
    nodeClientTitle: "Client Device",
    nodeClientDesc: "Phone or tablet browser (PWA) or Naro desktop on another computer using HTTPS API & SSE.",
    nodeRelayTag: "IN-MEMORY ONLY",
    nodeRelayTitle: "Naro Go Relay",
    nodeRelayDesc: "Holds only in-memory device routing and online status; multiplexes requests by ID and cleans up immediately.",
    nodeHostTag: "TARGET HOST",
    nodeHostTitle: "Host Computer",
    nodeHostDesc: "Runs Naro desktop and local CLIs, verifies pairing codes, reads workspace files, and keeps all history.",
    pipeHttps: "HTTPS / SSE",
    pipeHttpsSub: "Same-Origin",
    pipeWss: "Single WSS",
    pipeWssSub: "Outbound Only",

    simStream: "Send Prompt (SSE)",
    simApproval: "Concurrent Approval",
    simStop: "Instant Stop",
    simHeartbeat: "25s Idle Heartbeat",
    telDisk: "Relay Disk Writes",
    telPorts: "Inbound Ports",
    telWss: "Host WSS Count",
    simLogDefault: "Pick a scenario above to watch requests multiplex in memory over a single WSS tunnel.",
    simLogStream:
      "[req_id=7c02] POST /api/runs → Go relay multiplexes over WSS → Host runs CLI → SSE streams back (0 B on disk)",
    simLogApproval:
      "[req_id=7c09] While SSE is still streaming, approval rides the same WSS concurrently without blocking",
    simLogStop:
      "[req_id=7c14] POST /api/runs/{id}/stop → Host halts active turn immediately; queued follow-ups marked unsent",
    simLogHeartbeat:
      "[keep-alive] Idle host exchanges lightweight 25s heartbeats; reconnects with jittered backoff, never replaying writes",
    honestTitle: "Plain transparency: ",
    honestText:
      "The Naro Go relay persists zero user data. In the official deployment, the Nginx reverse proxy keeps standard access logs (source IP and request path)—or self-host your own private relay in one command.",

    featuresKicker: "BUILT FOR REAL WORKFLOWS",
    featuresTitle: "Not a toy remote—a complete coding workspace",
    featuresDesc: "Every ergonomic detail you rely on at your desk, carefully carried over to web and mobile.",

    featQueueTitle: "Queue, Steer & Withdraw to Edit",
    featQueueDesc:
      "Send follow-ups mid-reply: Queue executes sequentially after the current turn, or Steer injects directly into the active turn. Withdraw queued items with images and switch back to your draft anytime.",
    featQueueRow1: "Add reconnect backoff unit tests",
    featQueueBadge1: "Steer",
    featQueueRow2: "Update Nginx snippet in README.en.md",
    featQueueBadge2: "Withdraw",

    featGitTitle: "Full Remote Git Management",
    featGitDesc:
      "Built right into the top bar on both desktop and web: inspect status & diffs, stage/unstage, discard changes, commit (with Amend), create or switch branches, and Fetch / Pull / Push remotely.",

    featUsageTitle: "Token Usage & Account Limits",
    featUsageDesc:
      "Track 24-hour, 7-day, and 30-day token usage across Codex, Claude Code, and Antigravity (including cache reads/writes), plus live account allowances and Codex quota resets.",

    featPreviewTitle: "Read-Only File Preview & Image Paste",
    featPreviewDesc:
      "Click file links in replies to preview Markdown or UTF-8 source up to 2 MiB. Paste screenshots with ⌘V / Ctrl+V or attach up to 4 PNG/JPEG/GIF/WebP images (12 MiB total).",

    featPairTitle: "QR Pairing & Multi-Device Switching",
    featPairDesc:
      "Scan a QR code from your phone or pair another desktop with an 8-character code. Add to Home Screen on iOS Safari and hop between multiple host computers on one relay.",
    pairCodeLabel: "8-char Host Pairing Code",
    pairCodeSub: "Verified locally by host; relay stores no credentials",

    featCraftTitle: "Engineered with Native Restraint",
    featCraftDesc:
      "Lightweight Rust (eframe/egui) desktop app paired with a zero-framework web workbench. Trilingual UI, synced Light/Dark/System appearance, and per-CLI HTTP/HTTPS proxy settings.",
    craftPills: [
      "简体中文 · EN · 日本語",
      "Light / Dark / System",
      "CLI Proxy Support",
      "macOS & Windows",
      "Local Web 127.0.0.1:4318",
      "Zero-Framework Web UI"
    ],

    stepsKicker: "THREE MINUTES TO FLOW",
    stepsTitle: "Connect your main workstation in three steps",
    step1Title: "Launch Naro on your host",
    step1Desc: "Sign in to codex, claude, or agy locally, then open Naro.app on macOS or Naro.exe on Windows.",
    step2Title: "Point to your Go relay",
    step2Desc: "Enter the relay HTTPS URL and shared token under Device Connections to establish one outbound WSS.",
    step3Title: "Scan QR or pair, pick up live",
    step3Desc: "Scan from your phone browser or enter the device URL and 8-char code on another computer's Naro app.",

    deployKicker: "SELF-HOST IN SECONDS",
    deployTitle: "Single-binary Go relay, zero database",
    deployDesc: "No Postgres, no Redis. Run a single Go binary directly or deploy the minimal scratch container with Docker Compose.",
    codeTabGo: "Go Relay",
    codeTabDocker: "Docker",
    codeTabBuild: "Build Source",
    copyLabel: "Copy",
    copiedLabel: "Copied",
    envPublicDesc: "Public HTTPS origin of your relay (scheme, host, optional port)",
    envTokenDesc: "Shared 32–512 ASCII secret token with no whitespace",
    envAddrDesc: "Relay listen address, defaults to 127.0.0.1:7319",
    envTlsDesc: "Optional cert & key files to serve HTTPS directly from Go",

    closingTitle: "Step away from your desk. Keep your momentum.",
    closingDesc: "Naro is open source under the MIT License for macOS and Windows. Download a release or self-host today.",
    closingPrimary: "Download on GitHub Releases",
    closingSecondary: "Explore Source Code",
    footerMeta: "MIT License · Bundled Noto Sans CJK SC (SIL OFL 1.1)",
    footerZh: "简体中文 README",
    footerEn: "English README",
    footerJa: "日本語 README",

    engines: {
      codex: {
        sessionTitle: "Harden Go relay WSS heartbeat & backoff",
        cwd: "~/WorkSpace/naro-link",
        model: "gpt-5.4 · High",
        fast: true,
        perm: "Request approval",
        userMsg: "Audit reconnect logic in relay_ws.go to guarantee write requests never replay.",
        assistantHtml:
          'Reviewed <span class="file-chip">relay_ws.go</span>. Added jittered exponential backoff and immediate cleanup of <code>pendingRequests</code> on disconnect so side-effecting calls never duplicate.',
        phoneCmd: "go test ./... -race -count=1",
        phoneSummary: "Codex prepared the patch and wants to run Go race detector tests."
      },
      claude: {
        sessionTitle: "Refine sidebar project groups & file preview",
        cwd: "~/WorkSpace/naro-link",
        model: "claude-sonnet-4-6",
        fast: false,
        perm: "CLI permission",
        userMsg: "Fall back to source-only preview for Markdown files larger than 256 KiB.",
        assistantHtml:
          'Added the 256 KiB guard in <span class="file-chip">src/file_preview.rs</span> and <span class="file-chip">public/app.js</span> while preserving the 2 MiB file cap and copy actions.',
        phoneCmd: "cargo test --locked file_preview",
        phoneSummary: "Claude Code updated Markdown fallback and asks to run unit tests."
      },
      antigravity: {
        sessionTitle: "Polish mobile PWA theme sync & safe areas",
        cwd: "~/WorkSpace/naro-link",
        model: "gemini-3.5-pro",
        fast: true,
        perm: "ACP mode",
        userMsg: "Check iOS Safari standalone bottom safe-area spacing and cross-tab theme sync.",
        assistantHtml:
          'Verified <code>env(safe-area-inset-bottom)</code> in <span class="file-chip">public/style.css</span> and the <code>storage</code> listener in <span class="file-chip">public/theme.js</span> for instant tab sync.',
        phoneCmd: "node scripts/theme-e2e.mjs",
        phoneSummary: "Antigravity finished safe-area adjustments and is ready for E2E checks."
      }
    }
  },

  ja: {
    pageTitle: "Naro — ローカル AI コーディングセッションをどの端末からでも",
    navWorkspace: "ワークスペース",
    navRelay: "リレー構成",
    navFeatures: "主な機能",
    navDeploy: "セルフホスト",
    navDownload: "ダウンロード",
    eyebrow: "Codex · Claude Code · Antigravity 対応",
    heroLine1: "セッションは自宅の PC に、",
    heroLine2Prefix: "続きは",
    heroLine2Accent: "どこからでも。",
    heroSubtitle:
      "接続先 PC でネイティブ CLI を実行し、別の PC の Naro デスクトップやスマホ・タブレットのブラウザーからシームレスに再開。Go リレーはメモリ転送のみでディスクに一切保存しません。",
    ctaDownload: "デスクトップ版を入手 (macOS / Windows)",
    ctaDeploy: "Go リレーを配備",
    metaRust: "Rust ネイティブアプリ",
    metaGo: "ディスク非保存 Go リレー",
    metaMit: "MIT ライセンス · 受信ポート不要",

    stageWss: "単一 WSS 接続 · ディスク書込 0",
    tabChat: "セッション",
    tabGit: "Git 管理",
    tabUsage: "使用量",
    sidebarProjects: "プロジェクト",
    sidebarQuotaLabel: "過去 24h トークン",
    sidebarQuotaSub: "キャッシュ読込・書込を含む",
    phoneConnected: "接続中 · Mac Studio",
    phoneApprovalTitle: "実行の承認待ち",
    phoneApproveBtn: "許可する",
    phoneSteerBtn: "応答を誘導",
    phoneToastIdle: "WSS 接続中 · 18ms",
    phoneToastApproved: "✓ 承認を送信しました (req_id #a41f)",
    phoneToastSteered: "✓ 実行中のターンへ誘導しました",
    desktopSteerNotice: "iPhone から · 応答を誘導：再接続の単体テストも追加して",
    desktopApprovedNotice: "iPhone から · コマンド実行を承認、ストリームを継続中",
    composerPlaceholder: "Enter で送信 · Shift+Enter で改行 · ⌘V で画像貼付",
    composerMode: "キュー / 誘導",

    gitActionStage: "ステージ",
    gitActionCommit: "コミット (Amend)",
    gitActionPush: "Push",
    usage24h: "過去 24 時間",
    usage7d: "過去 7 日間",
    usage30d: "過去 30 日間",

    enginesKicker: "NATIVE CLI ENGINES",
    enginesTitle: "3 つのエージェントにネイティブ接続",
    enginesDesc:
      "独自形式に閉じ込めません。会話履歴はすべて接続先 PC の各 CLI が管理するため、端末とデスクトップ、スマホを自由に行き来できます。",
    codexDesc:
      "app-server 経由で既存セッションをプロジェクト別に表示。推論強度、稲妻の高速モード切替、4 つの権限プリセット、利用枠リセットに対応。",
    codexTags: ["app-server", "プロジェクト整理", "高速モード", "枠リセット"],
    claudeDesc:
      "ローカルの JSONL 履歴を直接読み込み、--resume で元のセッションを継続。CLI の権限モードとプラン利用枠をそのまま扱えます。",
    claudeTags: ["JSONL 履歴", "--resume 継続", "CLI 権限", "利用枠確認"],
    agyDesc:
      "ACP 経由で Antigravity セッションを読み込み・再開。デスクトップ版のデータディレクトリや SQLite 履歴集計、CLI 実行モードに対応。",
    agyTags: ["ACP 接続", "SQLite 集計", "実行モード", "デスクトップ連携"],

    relayKicker: "ZERO-PERSISTENCE ARCHITECTURE",
    relayTitle: "データを残さない Go リレー、1 本の WSS で多重化",
    relayDesc:
      "接続先 PC は受信ポートを開放せず、リレーへ 1 本の WSS を能動的に張るだけ。メッセージ・画像・ファイルプレビュー・ペアリング記録は一切ディスクに書き込まれません。",
    nodeClientTag: "ACCESS CLIENT",
    nodeClientTitle: "アクセス端末",
    nodeClientDesc: "スマホ・タブレットのブラウザーや別 PC の Naro デスクトップから HTTPS API と SSE で接続。",
    nodeRelayTag: "IN-MEMORY ONLY",
    nodeRelayTitle: "Naro Go リレー",
    nodeRelayDesc: "メモリ上でルーティングとオンライン状態のみ保持。リクエスト ID で並行転送し完了即解放。",
    nodeHostTag: "TARGET HOST",
    nodeHostTitle: "接続先 PC",
    nodeHostDesc: "Naro と各 CLI を実行。ペアリングコードを検証し、作業ディレクトリと履歴をローカル管理。",
    pipeHttps: "HTTPS / SSE",
    pipeHttpsSub: "同一オリジン",
    pipeWss: "単一 WSS",
    pipeWssSub: "送信方向のみ",

    simStream: "追加メッセージ (SSE)",
    simApproval: "応答中の並行承認",
    simStop: "即時停止",
    simHeartbeat: "待機中 25 秒ハートビート",
    telDisk: "リレー書込量",
    telPorts: "開放ポート",
    telWss: "接続先 WSS 数",
    simLogDefault: "上のシナリオをクリックすると、単一 WSS 上でのメモリ多重転送を確認できます。",
    simLogStream:
      "[req_id=7c02] POST /api/runs → Go リレーが WSS へ多重転送 → PC で CLI 実行 → SSE で逐次返信（保存 0 B）",
    simLogApproval:
      "[req_id=7c09] 逐次応答中でも承認リクエストを同じ WSS で並行転送し、ブロッキングなしで即応答",
    simLogStop:
      "[req_id=7c14] POST /api/runs/{id}/stop → 実行中ターンを即停止し、キュー内の残メッセージは未送信扱い",
    simLogHeartbeat:
      "[keep-alive] 待機中は 25 秒ごとに軽量ハートビートのみ送受信。切断後はジッター付きバックオフで自動再接続",
    honestTitle: "透明性について：",
    honestText:
      "Naro の Go リレー自体はユーザーデータを永続化しません。公式配備の Nginx リバースプロキシは接続元 IP やリクエストパスを含むアクセスログを記録します。専用リレーも数分でセルフホスト可能です。",

    featuresKicker: "BUILT FOR REAL WORKFLOWS",
    featuresTitle: "スマホでも妥協しない、本格コーディング環境",
    featuresDesc: "デスクで当たり前のように使う操作感を、ブラウザーとモバイルへ丁寧に届けます。",

    featQueueTitle: "キュー追加・応答誘導・取り消して編集",
    featQueueDesc:
      "応答中でも続けて指示を送れます。順番に送る「キューに追加」や、実行中ターンへ差し込む「応答を誘導」に対応。キューから取り消して編集し、書きかけの下書きへ戻ることも可能です。",
    featQueueRow1: "WSS 再接続バックオフのテストを追加",
    featQueueBadge1: "応答を誘導",
    featQueueRow2: "README.ja.md の Nginx 設定を更新",
    featQueueBadge2: "取り消して編集",

    featGitTitle: "リモート対応の Git 管理",
    featGitDesc:
      "デスクトップ・Web の上部バーからステータスと差分確認、ステージ/解除、変更破棄、コミット（Amend 対応）、ブランチ切替、Fetch / Pull / Push まで完結します。",

    featUsageTitle: "トークン使用量とアカウント上限",
    featUsageDesc:
      "過去 24 時間・7 日間・30 日間の合計とエンジン別消費（キャッシュ読込・書込込み）を集計。各アカウントの残り利用枠や Codex 枠リセットも確認できます。",

    featPreviewTitle: "ファイルプレビューと画像添付",
    featPreviewDesc:
      "応答内のファイルリンクから Markdown 整形/ソース切替で閲覧（最大 2 MiB）。⌘V / Ctrl+V でスクリーンショットや画像を 1 回 4 枚・合計 12 MiB まで添付できます。",

    featPairTitle: "QR ペアリングと複数 PC 切替",
    featPairDesc:
      "接続先 PC の QR コードをスマホで読み取るか、8 桁コードで別 PC とペアリング。iPhone はホーム画面に追加して PWA として利用でき、複数 PC を自在に切り替えられます。",
    pairCodeLabel: "8 桁ペアリングコード",
    pairCodeSub: "接続先 PC が直接検証・リレーに認証情報を保存しません",

    featCraftTitle: "ネイティブ品質の細部設計",
    featCraftDesc:
      "Rust (eframe/egui) の軽快なデスクトップ版とフレームワーク不要の Web 版。日中英 3 言語切替、ライト/ダーク/システム外観同期、CLI 向け HTTP/HTTPS プロキシ設定に対応。",
    craftPills: [
      "简体中文 · EN · 日本語",
      "ライト / ダーク / システム",
      "CLI プロキシ設定",
      "macOS & Windows",
      "ローカル Web 127.0.0.1:4318",
      "軽量ネイティブ UI"
    ],

    stepsKicker: "THREE MINUTES TO FLOW",
    stepsTitle: "3 ステップで、自宅の作業環境を手元に",
    step1Title: "接続先 PC で Naro を起動",
    step1Desc: "codex / claude / agy にログインし、macOS は Naro.app、Windows は Naro.exe を開きます。",
    step2Title: "リレー URL とトークンを設定",
    step2Desc: "「デバイス接続 → この PC のペアリング」に入力して接続。単一の送信 WSS が確立されます。",
    step3Title: "QR またはコードで即再開",
    step3Desc: "スマホのブラウザーで QR をスキャンするか、別 PC の Naro にアドレスと 8 桁コードを入力します。",

    deployKicker: "SELF-HOST IN SECONDS",
    deployTitle: "単一バイナリの Go リレーを数分で配備",
    deployDesc: "データベースも Redis も不要。単一の Go バイナリでも、Docker / Docker Compose の軽量コンテナでもすぐに配備できます。",
    codeTabGo: "Go リレー",
    codeTabDocker: "Docker",
    codeTabBuild: "ソースビルド",
    copyLabel: "コピー",
    copiedLabel: "コピー完了",
    envPublicDesc: "リレーの公開 HTTPS オリジン（スキーム・ホスト・任意ポート）",
    envTokenDesc: "リレーと接続先 PC で共有する 32〜512 文字のランダムトークン",
    envAddrDesc: "待ち受けアドレス（既定値 127.0.0.1:7319）",
    envTlsDesc: "任意：Go リレーから直接 HTTPS を提供する場合の証明書と秘密鍵",

    closingTitle: "デスクを離れても、思考は止めない。",
    closingDesc: "Naro は MIT ライセンスのオープンソースソフトウェアです。macOS / Windows 版を入手して始めましょう。",
    closingPrimary: "GitHub Releases から入手",
    closingSecondary: "ソースコードを見る",
    footerMeta: "MIT ライセンス · 同梱フォント Noto Sans CJK SC (SIL OFL 1.1)",
    footerZh: "简体中文 README",
    footerEn: "English README",
    footerJa: "日本語 README",

    engines: {
      codex: {
        sessionTitle: "Go リレーの WSS ハートビートと再接続を強化",
        cwd: "~/WorkSpace/naro-link",
        model: "gpt-5.4 · High",
        fast: true,
        perm: "承認を要求",
        userMsg: "relay_ws.go の再接続処理を確認して、書き込みリクエストが重複実行されないようにして。",
        assistantHtml:
          '<span class="file-chip">relay_ws.go</span> を確認しました。ランダムジッター付きバックオフを導入し、切断時に <code>pendingRequests</code> を即解放して重複送信を防止します。',
        phoneCmd: "go test ./... -race -count=1",
        phoneSummary: "Codex がパッチを作成し、Go のレース検出テストの実行許可を求めています。"
      },
      claude: {
        sessionTitle: "サイドバーのプロジェクト整理とファイルプレビュー改善",
        cwd: "~/WorkSpace/naro-link",
        model: "claude-sonnet-4-6",
        fast: false,
        perm: "CLI 権限モード",
        userMsg: "256 KiB を超える Markdown ファイルはソース表示にフォールバックさせて。",
        assistantHtml:
          '<span class="file-chip">src/file_preview.rs</span> と <span class="file-chip">public/app.js</span> に 256 KiB しきい値を追加しました。2 MiB 上限とコピー機能も維持しています。',
        phoneCmd: "cargo test --locked file_preview",
        phoneSummary: "Claude Code が修正を完了し、単体テストの実行を待機しています。"
      },
      antigravity: {
        sessionTitle: "モバイル PWA の外観同期とセーフエリア調整",
        cwd: "~/WorkSpace/naro-link",
        model: "gemini-3.5-pro",
        fast: true,
        perm: "ACP 実行モード",
        userMsg: "iOS Safari のホーム画面追加時の下部セーフエリアとタブ間の外観同期を確認して。",
        assistantHtml:
          '<span class="file-chip">public/style.css</span> の <code>env(safe-area-inset-bottom)</code> と <span class="file-chip">public/theme.js</span> の <code>storage</code> 監視を確認し、即時同期を調整しました。',
        phoneCmd: "node scripts/theme-e2e.mjs",
        phoneSummary: "Antigravity が調整を完了し、テーマ E2E テストの実行準備が整いました。"
      }
    }
  }
};
