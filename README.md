# Usage Control

Track your AI coding subscriptions from the Windows and Linux system tray.

Usage Control shows how much of your AI coding plans you have used: session and weekly limits,
credits, and local spend, all in one popup that opens when you click the tray icon.

> **Unofficial port.** Usage Control is an independent Windows and Linux port of
> [OpenUsage](https://github.com/robinebers/openusage) by Robin Ebers, which is a native macOS app.
> It is not the official OpenUsage, and it is not affiliated with or endorsed by its author.
> The source code is reused under the MIT license; the OpenUsage name and logo are not used,
> following the upstream [trademark policy](https://github.com/robinebers/openusage/blob/main/TRADEMARK.md).

## Status

Under active development. The port follows the upstream Swift edition (v0.7.12) feature by feature:

| Stage | Scope |
|---|---|
| 1 | Tray icon, popup anchored to the tray, footer, light and dark themes |
| 2 | Claude and Codex limits, pacing, local spend, Total Spend ring |
| 3 | Cursor, Grok, OpenCode, usage trend, per-model breakdown |
| 4 | Customize and Settings |
| 5 | Copilot, Antigravity, Devin, Ollama, OpenRouter, Z.ai |
| 6 | CLI, local HTTP API, proxy, quota notifications, global shortcut, launch at login |
| 7 | Windows installer, Linux `.deb` and `.AppImage` |

## Stack

- [Tauri 2](https://tauri.app/): a Rust core and the operating system's own web view.
- Rust core: credential readers, provider clients, local log scanners, model pricing, caching.
- React and TypeScript for the popup interface.

## License

[MIT](LICENSE). Original work copyright Robin Ebers; port copyright Bui Dang Minh.
