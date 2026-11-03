# Master Chat

One small, locked-down desktop app for Gmail, Outlook, Messenger, Zalo, WhatsApp,
Telegram, Discord and Gemini. Built with Tauri v2 and the system's own web view (WebView2 on
Windows), with plain HTML/CSS/JS for the sidebar: no framework and no bundler.

## Features

- **Sidebar:** right-click an account for Reload, Edit name and color, Notifications,
  Zoom, Move up/down and Remove; drag to reorder; optional names under the icons.
- **Overview** (the start screen): every account's unread count and the latest messages
  from all of them, kept in memory only.
- **Notifications** as Windows notifications; clicking one opens that account and chat.
  Messenger and Gmail are read by the app itself (sender and preview), since Messenger
  only notifies by Web Push, which WebView2 lacks; other services get a plain notice if
  their unread count rises without one.
  Per-account mute, Do Not Disturb (moon button or tray), and an option to hide the
  message text.
- **Tray and taskbar:** the unread total shows on the taskbar button and tray tooltip;
  closing the window keeps the app in the tray. Optional start with Windows, minimized.
- **Theme:** System, Light or Dark, for the sidebar and every service that follows it.
- **Messenger** shows only the chats (adapted from [fMess](https://github.com/toanbku/fmess),
  MIT): no Facebook bar or feed, a resizable chat list, the cursor in the message box
  when a chat opens, an accurate unread count, and optional "hide that I'm typing".
- **Outlook** unread count, read from the Inbox in the folder list.
- **Links** open in your browser without Facebook's and Google's redirects and tracking
  parameters.
- **App lock (optional):** a PIN to open the app, auto-lock after inactivity, and
  **Ctrl+Alt+H** to hide the window (and lock it) instantly.
- **Updates** install themselves from GitHub releases, signed and verified.

## Security model

- **Each account has its own browser profile** (`%LOCALAPPDATA%\dev.masterchat.desktop\profiles\<id>`),
  so cookies and logins never mix. Removing an account deletes its profile.
- **Service pages can't manage the app.** Only the local sidebar may call the app's
  commands (`src-tauri/capabilities/shell.json`). Service pages may call just one,
  `notify` (`capabilities/services.json`), which checks which account is calling and
  rate-limits it. Every command also checks its caller in code.
- **Navigation allowlist.** Each service's webview may only load HTTPS pages on its own
  domains (`src-tauri/src/services.rs`); everything else opens in your browser. The one
  exception is sign-in for school and work accounts: when Google's or Microsoft's sign-in
  page sends you to your organization's own login page, that page loads in the app until
  sign-in returns to the service. Such pages can't call the app at all.
- **You sign in on the services' own pages.** The app never sees passwords, has no
  server and sends no analytics.
- **The PIN** is stored only as a salted PBKDF2-SHA256 hash (300,000 rounds); wrong
  guesses slow down after five tries. While locked, the sidebar shows no accounts and
  notifications don't show message text.
- Strict CSP and a frozen JS prototype for the sidebar; no devtools in release builds.

## Run

You need Node.js, Rust (stable) and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
npm install
npm run dev     # run
npm run build   # installer in src-tauri/target/release/bundle/nsis
```

Tests: `cargo test` in `src-tauri/`.

## Releasing updates

Updates are signed with a private key kept outside the project, in
`~/.tauri/master-chat.key`; the app only installs updates signed with it. Keep a
backup of that file: without it, installed copies can't be updated.

1. Put the project on GitHub and set the endpoint in `src-tauri/tauri.conf.json`
   (`plugins.updater.endpoints`) to
   `https://github.com/<you>/<repo>/releases/latest/download/latest.json`.
2. Add the repository secret `TAURI_SIGNING_PRIVATE_KEY` with the key file's contents.
   The repository must be public, so installed copies can download `latest.json`.
3. Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and
   `package.json`, then push a tag such as `v0.2.0`. The workflow builds a draft
   release; publish it and installed copies offer the update.
