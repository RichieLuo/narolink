# Naro

[简体中文](README.md) · **English** · [日本語](README.ja.md)

Pick up your Codex, Claude Code, and Antigravity sessions from a computer, phone, or tablet. Naro runs the CLI on the host computer. Connect from the Naro desktop app on another computer or from the web on a phone or tablet. The host initiates the connection to the Go relay and needs no inbound ports. Each CLI manages its own session history. The relay forwards requests and responses in memory.

## Get started

Install and sign in to the CLI you want to use on the host computer (`codex`, `claude`, or `agy`). On macOS, extract the archive and open `Naro.app`. On Windows, run `Naro.exe`. You can also build from source using the instructions below.

Open an existing session from the sidebar, or choose an engine and a working directory on the host to start a new session. Naro groups Codex sessions by registered project and includes older sessions when it can identify their project from the working directory. Click **＋** beside a project to start a session there. For a project with multiple folders, choose the folder to work in. Claude Code and Antigravity can use previously opened working directories.

- The model menu lists the models and reasoning effort levels from the CLI. For models that support fast mode, click the lightning bolt next to the menu to switch speeds. A filled bolt means fast mode, and an outline means standard speed. If you change models during a run, the next turn uses the settings selected when you send the message.
- Choose Simplified Chinese, English, or Japanese under **Settings → General → Interface language** on desktop or web. The change applies immediately and each client remembers its own choice. Settings are also available before pairing on the web.
- Choose **Light / Dark / System** under **Settings → General → Appearance**. System is the default and follows the current device. Changes apply immediately; desktop and web remember their own choices, and browser tabs share the web preference.
- Open **Usage → Token usage** to view total and per-engine counts for the past 24 hours, 7 days, or 30 days. Input counts include cache reads and writes. Naro uses the connected computer's CLI history, including archived Codex sessions and Antigravity's SQLite sessions. **Account limits** shows the remaining provider allowances and reset times.
- Send follow-ups during a reply on desktop or web. **Queue** sends messages in order after the current turn. **Steer** sends a new or queued message into the current turn using its existing model (supported for Codex, Claude Code, and Antigravity). Change the send mode in **Settings → Sessions**. Each client remembers its selection and defaults to Queue. Clear the draft during a run to reveal **Stop**. Stopping marks the remaining queued messages as unsent.
- Select **Withdraw to edit** in the queue above the composer to bring a message and its images back into the editor. Use **Switch draft** to return to a message you were already writing. Resending puts the edited message at the end of the queue. The copy icon works on your messages and assistant replies.
- On desktop, click a local file link in an assistant reply to open a read-only preview on the right, with copy, reload, and **Show in folder** actions. Markdown renders by default; switch between **Preview / Source**. Other UTF-8 text files show source. The limit is 2 MiB per file; Markdown larger than 256 KiB shows source only. On smaller windows, the session list hides while previewing and returns when the preview closes.
- Press **Enter** to send or **Shift+Enter** for a new line. Paste text, screenshots, or images with **⌘V** (macOS), **Ctrl+V**, or the right-click menu.
- Attach up to four PNG, JPEG, GIF, or WebP images per message, with a combined source size of 12 MiB. The image picker is also available on desktop and web. The host validates and normalizes the images before passing them to the CLI. The browser can preview images from recent runs.

To use the browser workspace on the host itself, open **Device connections → Open local web app** (**设备连接 → 打开本机网页**). The default address is `http://127.0.0.1:4318`.

## Deploy the Go relay

Build the relay with Go 1.23 or later. Set its public HTTPS address and connection token on the server. The example uses OpenSSL to generate a random token. Replace the example domain with your own and enter the same token on each host you want to connect. Keep the token in the relay service's environment configuration so hosts can reconnect after a restart.

```sh
go build -trimpath -ldflags="-s -w" -o narolink-relay .
export NAROLINK_PUBLIC_URL=https://naro.example.com
export NAROLINK_RELAY_TOKEN="$(openssl rand -hex 32)"
./narolink-relay
```

| Environment variable | Purpose |
| --- | --- |
| `NAROLINK_PUBLIC_URL` | Public HTTPS origin of the relay, consisting of the scheme, host, and optional port |
| `NAROLINK_RELAY_TOKEN` | Random token shared by the relay and the hosts. Use 32 to 512 ASCII characters with no whitespace |
| `NAROLINK_RELAY_ADDR` | Relay listen address. Defaults to `127.0.0.1:7319` |
| `NAROLINK_TLS_CERT`, `NAROLINK_TLS_KEY` | Optional. Set both the certificate and key files to serve HTTPS directly from Go |

If you use an HTTPS reverse proxy, preserve the original `Host` header, allow request bodies of at least 36 MiB, and set a response timeout of at least 45 seconds. Here is a Caddy configuration.

```caddyfile
naro.example.com {
    reverse_proxy 127.0.0.1:7319
}
```

## Connect from another device

1. On the host, open **Device connection** (**设备连接**), enter the relay's HTTPS address and connection token, and click **Connect** (**连接**). On Windows, **Save and connect** (**保存并连接**) stores the token in the system credential store for reconnection at launch.
2. Once the host is online, tap **Scan to connect** (**扫码连接**) in Naro on your phone and point the camera at the computer’s QR code to pair automatically. You can also scan with the phone’s camera app and open the link. Keep the host and Naro running.
3. Open **Manual pairing** (**手动配对**) on the host to find its device address and eight-digit pairing code. In Naro on another computer, go to **Device connection → Connect to another device** (**设备连接 → 连接另一台设备**), enter the address and code, and click **Pair and connect** (**配对并连接**). Only the host needs relay setup. Use **Return to this computer** (**返回本机**) to switch back to your own workspace. You can reconnect to the most recently paired computer while Naro stays open. The device address and pairing code also work in a browser.
4. One relay can serve multiple hosts. To return to a paired host, use **Switch device** (**切换设备**) in the browser sidebar or **Device list** (**设备列表**) on the relay homepage. On iPhone, use **Share → Add to Home Screen** in Safari.

To connect automatically when Naro starts, set `NAROLINK_RELAY_URL` and `NAROLINK_RELAY_TOKEN` on the host. Naro uses the hostname by default. Set `NAROLINK_DEVICE_NAME` to display a different name.

When upgrading from a single-device version, update both the relay and the host, then pair again using the new device address. Older desktop clients can still pair from the relay homepage.

## CLI settings

Codex reads its existing sessions through `app-server`. Claude Code reads local JSONL history and resumes with `--resume`. Antigravity reads and resumes sessions over ACP, using the corresponding data directory for sessions from the Antigravity desktop app.

Use **Permissions** (**权限**) next to the model button in the composer to set permissions for the next Codex or Claude Code turn. In Codex, the default **Request approval** (**请求批准**) allows workspace edits and asks you to approve edits outside the workspace or network access. **Approve for me** (**替我批准**) lets Codex review eligible requests. **Full access** (**完全访问**) uses your local account's file and network permissions. **Custom (config.toml)** (**自定义 (config.toml)**) uses the active Codex configuration for the project. Claude Code uses the permission modes supplied by its CLI. In Antigravity, use **CLI execution mode** (**CLI 执行模式**) next to the model button to choose from the session modes the CLI exposes over ACP. Set the Antigravity desktop app's local permission preset in the native app.

Set an HTTP or HTTPS proxy under **Settings → Network → CLI proxy** in the desktop app. It applies to CLI processes Naro launches after you save it, including runs started from other devices. Clear the address and save to inherit Naro's launch environment again.

If Naro cannot find a CLI, set `NAROLINK_CODEX_CLI`, `NAROLINK_CLAUDE_CLI`, or `NAROLINK_AGY_CLI` to the executable's absolute path in Naro's launch environment.

## Build and test from source

The desktop app requires Rust 1.89 or later. Building the relay requires Go 1.23 or later. The end-to-end test also requires Node.js.

**macOS** (with Xcode Command Line Tools installed)

```sh
sh scripts/package-macos.sh
go build -o dist/narolink-relay .
node scripts/e2e.mjs target/release/narolink dist/narolink-relay
```

The script writes `dist/Naro-<version>-macOS-<architecture>.zip`. For repeated local installs, set `git config --local narolink.macosSigningIdentity '<signing identity>'` to a keychain identity so macOS can retain removable-volume permission across updates. Use `NAROLINK_MACOS_SIGN_IDENTITY` to override it. Without a configured identity, the script signs ad hoc.

**Windows** (with the MSVC build tools installed, in Visual Studio Developer PowerShell)

```powershell
cargo build --release --locked
New-Item -ItemType Directory -Force dist
Copy-Item target/release/narolink.exe dist/Naro.exe
go build -o dist/narolink-relay.exe .
node scripts/e2e.mjs dist/Naro.exe dist/narolink-relay.exe
```

Run `cargo test --locked` and `go test ./...` for the Rust and Go test suites. The end-to-end test uses mock CLIs, one Go relay, and two independent Rust instances. It covers device pairing, request isolation, resuming by the original session ID, image transfer, switching models, stopping runs, and responding to approvals.

## Web API

The following endpoints are available after pairing. When connecting through the relay, prefix each path with `/devices/{device_id}`. The local web app uses the paths as shown. Write requests require a same-origin `Origin` header. Supported `provider` values are `codex`, `claude`, and `antigravity`.

| Method and path | Purpose |
| --- | --- |
| `GET /api/sessions/{provider}` | List sessions on the host |
| `GET /api/sessions/{provider}/{id}` | Read session metadata, messages, and the active run |
| `GET /api/models/{provider}` | List models with supported reasoning effort levels and fast mode availability. Add `?settings=true` to include permissions and collaboration modes |
| `GET /api/usage` | Read recorded token totals and per-engine counts for the past 24 hours, 7 days, and 30 days |
| `GET /api/activity` | List active and recently completed runs |
| `GET /api/runs` | List active runs |
| `POST /api/runs` | Start a run and return a summary with its ID. Provide `provider`, `cwd`, and `prompt`, with optional `session_id`, `project_id`, `images`, and model settings |
| `GET /api/runs/{id}` | Read the reply snapshot, run status, pending approvals and questions, and follow-up delivery status |
| `POST /api/runs/{id}/messages` | Send a follow-up with `mode` (`queue` or `steer`), `prompt`, and optional `images`. Queued messages can specify model settings for the next turn in `options` |
| `POST /api/runs/{id}/messages/{message_id}/steer` | Move a queued message into the active turn |
| `POST /api/runs/{id}/messages/{message_id}/withdraw` | Withdraw a pending message and return its `prompt` and Base64-encoded PNG `images`. Pass `display_name` on upload to keep each original filename |
| `GET /api/runs/{id}/images/{index}` | Fetch a normalized PNG from a recent run |
| `GET /api/previews/{key}` | Fetch an image preview referenced by a message |
| `POST /api/runs/{id}/stop` | Stop the run |
| `POST /api/runs/{id}/approval` | Submit `approval_id` and a Boolean `allow` |
| `POST /api/runs/{id}/answer` | Submit `question_id` and `answers`, a map from individual question IDs to arrays of selected options or custom text |

Each entry in `images` has a `name` and Base64-encoded `data`. Model settings include `model`, `reasoning_effort`, `fast_mode`, `permission_mode`, `approval_policy`, and Codex's `collaboration_mode` (`default` or `plan`). Codex supports the `request-approval`, `approve-for-me`, `full-access`, and `custom` permission presets. The selected preset determines the approval policy. Legacy requests can still supply a sandbox mode and `approval_policy`.

## License

Naro's source code is available under the [MIT License](LICENSE). The bundled Noto Sans CJK SC font is available under the [SIL Open Font License 1.1](assets/OFL.txt).
