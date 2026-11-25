# NoKey privacy policy

**Effective date:** 25 November 2026

This policy covers NoKey, the Windows hotkey manager, in both forms: the
Microsoft Store app and the `NoKey.exe` file from GitHub Releases.

## Short version

NoKey has no account, no telemetry, no analytics, no ads and no server of its
own. The developer receives none of your data. Everything NoKey stores stays on
your PC. NoKey connects to the internet only to do what you set it up to do:
send a Discord mention through your own bot and, in the GitHub version only,
check for updates when you ask.

## What NoKey stores on your PC

| Data | Why | Where |
|------|-----|-------|
| Settings, profiles and bindings, including the texts you ask NoKey to type | So your bindings work after a restart | `config.json` in the NoKey data folder |
| Discord bot token (only if you add one) | To send Discord mentions | `token.bin` in the NoKey data folder, encrypted with Windows DPAPI so that only your Windows account can decrypt it |
| A backup of an upgraded older config | So an upgrade can be undone | `config.v1-backup-<time>.json` in the NoKey data folder |

The NoKey data folder is `%APPDATA%\NoKey`. In the Microsoft Store app,
Windows normally keeps new files in that folder in the app's private storage
and removes them when you uninstall NoKey. If the GitHub version already
created `%APPDATA%\NoKey`, the Store app may use that folder too; delete it
yourself if you want everything gone. In the GitHub version, delete the folder
yourself to remove everything.

`config.json` is plain text. Do not put passwords or other secrets into texts
to type.

## Keyboard input

To recognize your shortcuts, NoKey uses Windows hotkeys (`RegisterHotKey`)
and a low-level keyboard hook. The hook looks at key presses only in memory, to
match them against the shortcuts you recorded, and forgets them right away.
NoKey never saves, logs or sends your keystrokes anywhere.

When you use the **Type text** action, NoKey types the text you saved into the
active window, using the Windows `SendInput` function.

## Network connections

- **Discord** (`discord.com`), only if you add your own bot token. NoKey uses
  the official Discord API to check the token, to list the servers and text
  channels your bot can see, to look up the channel and the user ID you
  entered (so the editor can show their names), and to send the mention
  message you configured. That data goes from your PC directly to
  Discord and is handled under
  [Discord's privacy policy](https://discord.com/privacy).
- **GitHub** (`api.github.com`, `github.com`), GitHub version only, and only
  when you press **Check for updates**. NoKey asks GitHub for the latest
  release of the repository set in Settings and, if you agree, downloads it.
  The Microsoft Store app has no built-in updater: the Store updates it.
- **Open website** actions open the address in your default browser. NoKey
  itself sends nothing.

NoKey makes no other network connections.

## Sharing

NoKey does not sell, share or transfer any data to the developer or to third
parties. The only data that leaves your PC is what you send to Discord through
your own bot, as described above.

## Start with Windows

If you turn on "Start with Windows", NoKey starts when you sign in. In the
Microsoft Store app this is a Windows startup task that you control in
**Settings → Apps → Startup**. In the GitHub version it is the `NoKey` value
under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

## Children

NoKey is a general-purpose utility, not directed at children, and collects no
personal information from anyone.

## Changes

If this policy changes, the new version will be published in this file with a
new effective date. The full history is in the repository's commit log.

## Contact

Questions or concerns: open an issue at
<https://github.com/Maestoo-of-69/NoKey/issues>.
