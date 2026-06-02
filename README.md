![Windows](https://img.shields.io/badge/platform-Windows-blue)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

# Claude Code Usage Bubble

A floating, draggable circular bubble that shows your Claude Code and/or
Codex usage on Windows — inspired by the floating "memory boost ball" UX
of 360 Security and IObit Advanced SystemCare.

Drop it anywhere on screen, drag it around, snap it to a monitor edge,
left-click for a panel with both your 5-hour and 7-day windows, right-click
for the menu.

## Acknowledgements

Inspired by [CodeZeno/Claude-Code-Usage-Monitor](https://github.com/CodeZeno/Claude-Code-Usage-Monitor),
which solves the same "how close am I to the Claude Code limit?" problem
with a horizontal taskbar widget. This project takes the UX in a different
direction — a floating, draggable circular bubble that the user can place
anywhere on screen — and is a clean-room implementation: the HTTP client,
provider polling, credential discovery, tray rendering, and
self-updater are all written from scratch against the same public APIs
(Anthropic, ChatGPT, GitHub Releases).

## What you get

- A circular floating bubble showing your current 5-hour Claude Code or
  Codex usage as a percentage and a colored progress ring
- Drag anywhere — the bubble snaps to monitor work-area edges when
  released
- Resize with `Ctrl + MouseWheel` on the bubble, or use **Controls** →
  **Make smaller / Make larger / Reset size** from the right-click menu
  (140–360 logical pixels)
- Left-click the bubble for an expanded panel with both **5h** and **7d**
  bars plus reset countdowns
- Right-click for refresh, displayed providers, update frequency, startup,
  updates, exit
- Optional system tray icons (one per enabled provider)
- Auto-hide when a fullscreen app is in the foreground (games, video,
  presentations) — reappears when you leave fullscreen

## Who this is for

Windows 10/11 users who already have **Claude Code (CLI or App) installed
and signed in on Windows**. Codex support is optional — install and sign in
to the Codex CLI, then enable Codex from the right-click **Providers** menu.

## Requirements

- Windows 10 or Windows 11
- Claude Code (CLI or App) installed and authenticated
- Optional: Codex CLI installed and authenticated, if you want Codex usage

## Install

### Download the latest release

Grab `claude-code-usage-bubble.exe` from the
[Releases page](https://github.com/tiennm99/claude-code-usage-bubble/releases/latest).
Put it anywhere on disk (e.g. `%LOCALAPPDATA%\ClaudeCodeUsageBubble\`) and
run it. The app self-updates from the same Releases feed.

**First-run note:** the binary is unsigned, so SmartScreen will show
"Windows protected your PC". Click **More info** → **Run anyway**. Code
signing is on the roadmap.

### Build from source

```powershell
git clone https://github.com/tiennm99/claude-code-usage-bubble
cd claude-code-usage-bubble
cargo build --release
```

The binary lands at `target/release/claude-code-usage-bubble.exe`.

## Use

Run `claude-code-usage-bubble.exe`. The bubble appears near the bottom-right
corner of your primary monitor on first launch. Drag it where you want it,
release to snap to the nearest edge if you let go close to one.

- **Left-click** the bubble to open the expanded panel (5h + 7d + countdowns)
- **Right-click** for refresh, providers, refresh frequency, "Start with
  Windows", controls, auto-update check (Disabled / Hourly / Daily / Weekly),
  manual "Check for updates", exit
- **Drag** anywhere — it floats on top of all other windows
- **Ctrl + MouseWheel** on the bubble, or **Controls** in the right-click
  menu, to resize it
- **Tray icon** (if enabled): left-click toggles the bubble visibility,
  right-click opens the same menu

### Providers

Use the right-click **Providers** menu to choose what is shown:

- **Claude Code** is enabled by default
- **Codex** can be enabled alongside Claude Code or shown by itself
- **OpenCode Go** is listed for future support; usage bars remain disabled
  until OpenCode exposes a stable quota source or console integration is
  explicitly enabled

When multiple providers are shown, each gets its own bubble that you can position
independently.

## Diagnostics

```powershell
claude-code-usage-bubble.exe --diagnose
```

This writes a log file to:

```text
%TEMP%\claude-code-usage-bubble.log
```

Settings are saved to:

```text
%APPDATA%\ClaudeCodeUsageBubble\settings.json
```

## Privacy and security

What the app reads:

- Your local Claude Code OAuth credentials from `~/.claude/.credentials.json`
- If Codex is enabled, your local Codex credentials from `$CODEX_HOME/auth.json`
  or `~/.codex/auth.json`

What the app sends over the network:

- Requests to Anthropic's Claude endpoints to read your usage
- Requests to ChatGPT's Codex usage endpoint, if Codex is enabled
- Requests to GitHub only if you use the app's update-check feature

What the app stores locally:

- Bubble position(s) per model
- Bubble size
- Polling frequency
- Last update check time
- Displayed provider preferences

What it does **not** do: send credentials to any third-party server, run a
backend service, collect analytics, upload your project files, or write to
your Codex `auth.json` directly.

## License

Apache License 2.0 — see [LICENSE](LICENSE).
