# NoKey

A lightweight native Windows hotkey manager. Create keyboard shortcuts and
assign actions to them: mention someone in Discord, open an app, a file, a
folder or a website — or several of these in a row.

- Native Win32 + Rust. No Electron, WebView, .NET or other runtime.
- One portable `NoKey.exe` (~830 KB). Runs from the system tray.
- Event-driven: no polling, no idle timers, no background network.
- 5 themes, 4 interface languages (including Hebrew right-to-left).

First-time Discord setup: see [SETUP.md](SETUP.md).

## Features

### Shortcuts

| Type | Examples | How it works |
|------|----------|--------------|
| Standard | `F8`, `Ctrl + Q`, `Ctrl + Shift + P` | `RegisterHotKey` (the keyboard hook for Release / Long Press / Repeat). |
| Simultaneous | `A + S`, `Q + W + E`, `Ctrl + Alt + F9 + F10` | Keyboard hook; all keys within the combo window (default 350 ms). |
| Sequence | `A → B`, `Ctrl → K → C`, `Q → Q → E` | Keyboard hook; each step within the sequence timeout (default 1000 ms). |

- Up to 3 shortcuts per binding, recorded by pressing them.
- Conflict warnings per profile. Priority 0–9 decides when one key press
  matches several bindings; one key event never runs two bindings.
- A standard shortcut must include Ctrl, Alt or Win unless it is a non-typing
  key (F1–F24, Pause, …), so normal typing is never blocked.

### Trigger modes

| Mode | Runs |
|------|------|
| Press | once when the shortcut goes down |
| Release | once when it is released |
| Double Tap | on the second press within the double-tap window (default 300 ms) |
| Long Press | after holding for the long-press time (default 600 ms) |
| Repeat while held | at once, then every interval while held, up to the run limit |
| Toggle repeat | first press starts repeating, next press stops it (limit always applies) |

Repeat modes: minimum interval 0.1 s, hard limit 1–1000 runs, stop on the
first error, cooldown still applies. **Discord pings can't use repeat modes**
(the original specification forbids automatic repeated mentions). There is no
separate "Hold" mode: no action has a state that could be held.

### Actions

- **Discord Ping** — one message `<@USER_ID>`; only that user is pinged
  (`allowed_mentions.users`), never `@everyone`, `@here` or roles.
- **Open Application** — `.exe`/`.lnk`; command shells and script hosts are refused.
- **Open File** (programs, scripts and shortcuts refused), **Open Folder**.
- **Open URL in Default Browser** — `http(s)://` only, via `ShellExecuteW`;
  new tab vs. window is decided by the browser.
- Up to 10 actions per binding, in order, with an optional pause between them
  and "stop when an action fails". No action runs shell commands.

### Profiles, conditions, safety

- Profiles Default, Gaming, Work, Discord (+ custom: add, rename, duplicate,
  delete). Only the active profile's bindings (and "All profiles") work.
  Switch from the main window, the tray, or a global profile-switch shortcut.
- Optional conditions per binding: active program (`chrome.exe`), window title
  contains, not in fullscreen apps — checked when the shortcut fires.
- **DISABLE ALL BINDS** button and the always-on, reserved **Ctrl + Alt +
  Pause** emergency switch. Nothing is deleted; press again to restore.
- Pause one binding for 15 min, 1 h or until restart.
- Cooldown per binding or global: 0.01 s (10 ms) – 3600 s, dot or comma,
  stored as whole milliseconds. Discord's `429` limits always apply on top.

### Interface

- Themes: Dark, Light, Contrast, System (follows Windows light/dark live),
  Jewish (blue and white, light geometric lattice in headers). Instant.
- Languages: Русский, English, עברית (right-to-left mirrored layout),
  Язык боли (a joke Russian translation). Instant; windows are re-created.
- Search, sort, "show all profiles", More menu / right-click (duplicate,
  delete, enable/disable, pause), tooltips, icon (emoji/letter), description,
  sound and notification per binding.
- Native menus and tray balloons keep the Windows style.

## Files and privacy

| What | Where |
|------|-------|
| Settings and bindings | `%APPDATA%\NoKey\config.json` (schema 2) |
| Bot token | `%APPDATA%\NoKey\token.bin` — Windows DPAPI, never shown or logged |
| Backup of an upgraded 0.1 config | `%APPDATA%\NoKey\config.v1-backup-<time>.json` |
| Downloaded updates | `%LOCALAPPDATA%\NoKey\update\` |
| Autostart | `HKCU\...\CurrentVersion\Run`, value `NoKey` (only when enabled) |

Old (0.1) settings are upgraded automatically: the original is copied first,
the result validated, then saved; no binding is lost. No telemetry or cloud.
The keyboard hook keeps keystrokes in memory only to match shortcuts.
`NOKEY_DATA_DIR` overrides the data folder (portable use, tests).

## Build and test

```powershell
cargo build --release      # → target\release\NoKey.exe (standalone)
cargo test                 # unit + integration tests
cargo clippy --all-targets
cargo test --test discord_live -- --ignored invalid_token   # needs internet
```

Requirements: Rust stable (MSVC) and Visual Studio Build Tools with the
Windows SDK. The hotkey tests inject F21–F24 with `SendInput`; Windows drops
injected input while an administrator app (e.g. Task Manager) is the active
window, so run them with a normal window in front.

## Architecture

```
src/
  app.rs             state, message loop, glue between engines and UI
  config.rs, config/ model (v2), migrate (v1→v2), timing ("0.01" s ⇄ ms)
  shortcut.rs        shortcut model, labels, validation, conflicts
  engine/            pure hotkey engine: matcher (down/up, priority),
                     trigger (modes), recorder
  actions.rs         action engine (worker thread, ordered steps)
  discord/ net/ security/ update/ services/
  i18n/              translations; strings in locales/*.json, compiled in
  win/               hotkeys, keyboard hook, tray, autostart, shell, context
  ui/                hidden host window, main window, editor (+ actions
                     tab), settings, themes, drawing helpers
locales/             en.json, ru.json, he.json, pain.json
```

The hook callback only updates in-memory state and posts a message — no
network or program start happens inside it. Hotkeys, tray and timers belong
to a hidden host window, so visible windows can be re-created freely.
Dependencies: `windows-sys`, `serde`, `serde_json`.

## Updates

Publish a GitHub release with `NoKey.exe` and `NoKey.exe.sha256`; in NoKey:
Settings → Updates → `owner/repository` → Check. NoKey asks before
downloading, verifies SHA-256 (again before installing), asks again, then
replaces the exe and restarts. Integrity only — the exe is not code-signed.

## Measured (release build, this PC)

Windows 11 x64, `NoKey.exe` 845 824 bytes; idle in the tray, 4 bindings,
keyboard hook active, `Get-Process` over 60 s: CPU 0 ms (below the ~15.6 ms
timer resolution), private memory ≈ 1.9 MB, working set ≈ 12.5 MB (mostly
shared Windows DLLs), 1 thread.

## Known limitations

- Keys don't reach the hook while an administrator app is focused (UIPI);
  standard shortcuts through RegisterHotKey still work.
- System keys like `Ctrl + Alt + Del`, `Win + L` can't be bound.
- Simultaneous and sequence shortcuts don't block the keys; standard ones are
  consumed. With RegisterHotKey a key is consumed even if a condition then
  skips the binding.
- The target user is entered by ID (member lists need a privileged intent).
- `config.json` is a normal user file; actions are re-validated before
  running, but a `.lnk` in Open Application runs whatever it points to.
- `src/app.rs` is ~1 100 lines (above the 800-line guideline); splitting the
  profile/settings glue into its own module is a planned cleanup.
