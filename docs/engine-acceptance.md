# Naro 0.1.16：原生引擎接入与验收记录

验收日期：2026-11-27。脚本路径已更新至当前目录结构。本文件区分已实现、真实账号验证、mock 验证和未完成项；**不是三家全部能力已完成的声明**。

## 环境与交付

- CLI：Claude Code 2.1.325、Codex 0.169.0、Antigravity CLI 1.5.18。
- 保留会话开始时已有的未提交改动；未提交或推送 Git。
- 验收时的 macOS 包：`dist/Naro-0.1.16-macOS-arm64.zip`，沿用本机配置的签名身份，签名校验成功。
- 线上中继：`https://naro.bato.chat`，镜像 `narolink-relay:0.1.16-20261127025908-native-engines`。
- 上线前独立容器预检、上线后 12 个网页资源 SHA-256 校验通过；线上 HTTPS 流式快照、增量、重连和停止通过。
- `/Applications/Naro.app` 已替换为 0.1.16，安装二进制与签名包完全一致。旧版本备份：`~/Library/Application Support/narolink/backups/Naro-before-0.1.16-20261127025908-native-engines.app`。
- 当前对话由旧 Naro 进程托管，不能强制退出。空闲更新任务已启动，状态为 `installed_waiting_for_idle`；任务全部空闲 8 秒后正常退出、重启并校验配对密钥、本机资源和远程在线状态。**写此记录时尚未验证重启完成**。
- 更新状态：`/tmp/naro-protocol-20261127/release/installation-status.json`。重启日志在同目录 `restart.log`、`restart-error.log`。LaunchAgent：`chat.bato.narolink.upgrade.0.1.16-20261127025908-native-engines`。

## 用户入口

- 桌面标题栏：**全文搜索**、**上下文**、已打开会话的 **会话操作**。
- 手机网页标题栏：**会话操作**，包含全文搜索、上下文输入及支持的原生操作。
- 压缩／Review 按钮将原生命令填入输入框，由用户确认发送。Antigravity 不显示未经证实的压缩快捷按钮。
- 运行回复中展示上下文、计划和逐文件 diff。侧栏用量统计保持原实现，不将历史消耗冒充上下文。
- 新入口使用共享中／英／日翻译。Service Worker 为 `narolink-shell-v73-native-engines`。

## Claude Code

### 已实现

- 移除普通会话的工具白名单、`disableAllHooks`、`--disable-slash-commands`、空 `--strict-mcp-config`；普通会话使用 CLI 默认原生工具及用户配置。标题生成等工具隔离请求仍使用 safe mode。
- 所有 `can_use_tool` 请求可进入现有审批流程，不再静默拒绝 Agent、任务、Skills、MCP 等原生工具。AskUserQuestion 保留结构化问答。
- Agent／Task 子代理沿用原工作事件；TodoWrite、TaskCreate／TaskUpdate 增加计划状态展示。
- ExitPlanMode 请求显示计划批准；批准后请求切换为手动审批，不自动使用 bypassPermissions。
- `rename_session`、`--resume --fork-session`；分支返回新原生 session ID。
- `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1`；按原生用户消息 UUID dry-run 和执行 `rewind_files`。明确仅恢复受跟踪文件，不回退对话；无检查点返回错误。
- `--worktree` 新会话、`--add-dir`、`--agent`；输入校验。worktree 的会话目录优先从原生索引读取。
- `claude agents --json` 活跃状态；busy 会话拒绝另起写入。idle 后续开新一轮不称作同轮接手。
- `get_context_usage` 查询，轮次前后和历史恢复后显示实际字段。

### 真实验证

- 原生 CLI 建会话 → Naro 发现 → 原 ID HTTP 续聊 → 原生编辑审批 → 编辑隔离文件成功。
- 隔离会话 `be20e265-45bb-452c-a070-ab2d3296ef03`；另一次为 `2affd286-de33-4c3b-b2d9-34f92d5b5db2`。
- 原生重命名和分支新 ID 成功，例如 `471eec36-4e88-4d4b-86f8-550d65c7929d`。
- 检查点 `949b40bc-2a83-4ac5-a625-36fa1f752ca2`：预览 1 文件、1 行增／删；执行后 `sample.txt` 从 `after\n` 恢复为 `before\n`，对话不变。
- 带提交的隔离 Git 仓库创建 `.claude/worktrees/naro-isolated-test`，Git worktree list 确认独立分支和目录。
- 实际上下文 10,732 / 1,000,000；原生 `/compact` 完成并返回更新数值。压缩后不保证数值一定下降。

### 边界／未完成

- 未实现、未验证正在执行一轮的同轮接手；只展示状态和空闲后续开。
- 文件 rewind 不等于对话 rewind，且不保证恢复 Bash／外部进程直接写入的文件；官方 SDK 文档明确此范围。
- 原生任务工具可执行、可展示，但 TaskCreate 的原生 task ID 与后续 TaskUpdate 的完整关联及历史任务状态恢复仍需完善。
- ExitPlanMode 已接入，尚未完成真实账号“子代理＋任务＋计划批准实施”整条组合验收。
- 检查点 UI 需要输入消息 UUID；未增加逐消息的一键回退选择器。

## Codex

### 已实现

- 普通 app-server 不再强制关闭 Hooks、Plugins、Apps 和 MCP。
- `thread/search` 全文搜索；`thread/searchOccurrences` 首个命中定位；可搜索归档。原项目归属逻辑保留。
- `thread/name/set`、`thread/archive`、`thread/unarchive`、`thread/fork`；归档后原 ID 使用 `thread/read` 回查，不依赖非归档列表。
- `turn/plan/updated` 显示 pending／inProgress／completed。
- `review/start`：未提交改动、指定 base branch。
- `thread/goal/set|get|clear`，显式 user origin，Token budget 和原生状态结果。
- `hooks/list`、`plugin/installed`、`app/list`，以及有效 feature 配置；各查询错误独立保留。
- `mcpServerStatus/list` 返回服务器、实际工具与授权状态；OAuth URL 返回后保留 app-server 等待回调；`config/mcpServer/reload` 原生重载。
- 命令／文件审批使用 `availableDecisions` 全量选项，结构化决定原样回传；旧 CLI 缺少该字段时保留原布尔回退。额外权限申请仍按 turn scope 批准。
- `thread/tokenUsage/updated` 和 `thread/compact/start`。旧会话读取原 rollout 最后一个 `event_msg/token_count` 的 last usage／model context window，不使用累计历史 tokens。
- 附加目录进入 runtime workspace roots 和显式 writableRoots。

### 真实验证

- 原生 `codex exec` 建会话 → Naro 发现 → 原 ID 续聊 → 实际编辑隔离文件成功。会话 `01a21924-42bd-7e92-9bdc-08a0874efa45`。
- 原生计划模式触发 request_user_input → Naro 回答 → 完成：`01a2192d-3a02-7b30-bb02-d45e34b263c4`。执行模式同类测试曾超时，已停止，不计通过。
- 重命名、fork 新 ID、Goal 设置／读取／清除及 2,000 Token 预算、归档／恢复完整 Naro HTTP 流程通过。
- `/compact` 在原 ID 完成，实际上下文从 30,623 / 475,000 更新到 28,985 / 475,000。
- `/review` 和 `/review main` 均完成，返回隔离文件审查结果。
- 隔离 CODEX_HOME 中 MCP `naro_fixture` 实际 connected，`naro_echo` 出现在工具列表，调用返回 `NARO_MCP_CALLED:verified`。原生 MCP reload 返回成功；Hooks／Plugins 列表返回原生结构。

### 边界／未完成

- 全文搜索 API 支持 cursor，但当前 UI 仅展示首批 50 个结果和首个命中，未实现全部分页与逐命中导航；很长历史未载入的命中仍需完整的目标 turn hydration。
- OAuth 登录需要用户及可用的真实 OAuth MCP 服务；保留回调进程已实现，但未做完整账号授权验收。
- 原配置 Apps 列表探测曾超时；展示错误，不宣称已可调用。
- 本次真实编辑未触发 Codex 命令审批；availableDecisions 各选项及单次回传以 mock 集成覆盖，未逐项完成真实授权缓存／策略修改验证。
- MCP reload 是原生配置重载，不承诺接管另一个 app-server 已运行连接。

## Antigravity

### 已实现

- 保留默认 customizations，不再强制 `--include-default-customizations=false`。
- ACP `tool_call`／`tool_call_update` 的 diff content 使用实际 path、oldText、newText；记录工具状态。调用 ID＋路径＋编辑内容去重。`session/load` 历史回放恢复保留 diff。
- `additionalDirectories`、`--agent` 及 ACP resource embedded context；文件大小／路径校验。
- CLI 与桌面版双索引合并；原 ID 根据原生 conversation 文件选择 `antigravity-cli` 或 `antigravity` 数据目录。
- ACP permission options 全部展示，保留 name、kind、optionId，选定值原样回传。
- ACP `usage_update.used/size` 上下文状态。

### 真实验证

- 原 CLI 建会话 → Naro 发现 → 原 ID 续聊 → `sample.txt` 编辑成功；重新加载历史恢复 `/private/.../sample.txt` 的 `before → after` diff，仅 1 份。会话 `fdc094dd-46a6-4981-9185-5c0cd270224f`。
- 原生问题 → Naro 选择 → 完成：`aa651f7d-64ee-446d-bd3d-523b298e508b`。
- 实际 ACP 上下文 12,158 / 1,048,576。初始化确实声明 embeddedContext 和 additionalDirectories。

### 原生限制／未完成

- 本机 ACP `session/set_name`、`session/fork`、`session/rollback`、`session/commands`、`session/agents` 均返回 `-32601 Method not found`。
- 官方 TUI 文档有 rename／fork／rewind／compact，不等于 ACP 提供等价控制。ACP `/context` 实测进入普通模型对话；`/compact` 未获得可确认的原生压缩完成标志。因此不提供伪装成功的快捷入口。
- `agy agents` 本机未返回可用列表；当前仅支持已知 Agent 名称输入，未完成实际命令／Agent 动态列表入口。
- 目录／embeddedContext 已接请求，尚未完成真实模型读取嵌入文件、模型切换后上下文及桌面来源新会话恢复的组合验收。
- pending diff 可能是提案；失败不会新增 diff，但已显示提案在后续失败时的显式撤销／状态标注还需完善。

## 测试结果与可复用入口

- `cargo test`：49 passed，0 failed。
- `go test -race ./...`、`go vet ./...`：通过。
- `cargo fmt --check`、`git diff --check`、JavaScript syntax：通过。
- `node tests/e2e.mjs <Naro binary> <relay binary>`：完整通过，包括三家原 ID、图片、中继、问题、审批、队列／引导、模型、权限、项目、用量与标题。
- `web/tests/native-ui-e2e.mjs`：Chromium 1280px／390px 上下文、计划、diff、rename、goal、compact 交互通过；不冒充原生 egui 自动化或真实手机设备测试。
- `web/tests/theme-e2e.mjs`：Chromium 主题、跨 tab、键盘、翻译、对比度与手机布局通过。
- `desktop/tests/native-acceptance.mjs`：需 `NARO_REAL_ACCEPTANCE=1`；真实账号、新隔离 Git 项目、原 CLI 建立会话、Naro 同 ID 续聊、自动处理原生问答／审批并检查文件结果。
- **真实账号三家测试使用本机 HTTP；生产 HTTPS 手机转发链路由 mock CLI 验收。尚未完成三家真实账号＋实际手机网页的整条联合验收，也未自动点击真实 egui 桌面窗口。**
- 本次原生协议与测试证据：`/tmp/naro-protocol-20261127/`，真实账号报告位于日志中列出的 `naro-real-acceptance-*` 隔离目录。临时日志可能被系统清理。

## 官方资料

- Claude SDK file checkpoints：<https://code.claude.com/docs/en/agent-sdk/file-checkpointing>
- Codex app-server：<https://developers.openai.com/codex/app-server>；以本机 `codex app-server generate-json-schema --experimental` 的完整请求／通知清单核对。
- Antigravity CLI reference：<https://antigravity.google/docs/cli/reference.md>；TUI 和 ACP 支持范围分别实测。

## 发布脚本

- `relay/scripts/deploy-relay.sh`：已知线上镜像前置检查、flock、防并发、独立容器预检、资源 hash 和失败回滚；不自行选择任意主机。
- `desktop/scripts/install-macos.py`、`desktop/scripts/open-macos.swift`：备份安装、签名／hash 校验、任务空闲后重启；中继凭据走 stdin，不进入命令行或发布包。
