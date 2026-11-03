# Setting up NoKey with Discord

NoKey sends mentions through **your own Discord bot**. You create the bot once
(about 5 minutes), add it to your server, and paste its token into NoKey.

Official docs used below: Discord Developer Portal
(<https://discord.com/developers/applications>), Message API
(<https://docs.discord.com/developers/resources/message>).

## 1. Create a Discord application and bot

1. Open <https://discord.com/developers/applications> and sign in.
2. Click **New Application**, give it a name (for example `NoKey`), accept the
   terms and click **Create**.
3. Open the **Bot** page in the left menu. A bot user is created with the
   application. You can set its name and avatar here.
4. **Privileged Gateway Intents:** leave all of them **off**. NoKey only uses
   the REST API and does not need them.

## 2. Get the bot token

1. On the **Bot** page click **Reset Token**, confirm, and copy the token.
2. Treat the token like a password: whoever has it controls the bot. Do not
   share it, post it or commit it anywhere. If it leaks, click **Reset Token**
   again – the old one stops working immediately.

## 3. Invite the bot to your server

1. Open **OAuth2 → URL Generator**.
2. Under **Scopes** tick **bot**.
3. Under **Bot Permissions** tick **View Channels** and **Send Messages**.
   Nothing else is needed.
4. Copy the generated URL, open it in the browser, choose your server and
   click **Authorize**. You need the *Manage Server* permission on that server.

Equivalent URL (replace `APP_ID` with the Application ID from **General
Information**; `3072` = View Channels + Send Messages):

```
https://discord.com/oauth2/authorize?client_id=APP_ID&scope=bot&permissions=3072
```

If the channel you want to use is private, also allow the bot (or its role)
to view and send messages in that channel's permission settings.

## 4. Connect NoKey

1. Start `NoKey.exe`, click **Settings** and open the **Discord** tab.
2. Paste the token into the field and click **Connect**.
3. NoKey checks the token with Discord. On success it shows
   **Connected as \<bot name\>** and saves the token encrypted with Windows
   DPAPI. The input field is cleared; the full token is never shown again.
4. Click **Save** or **Close**.

## 5. Choose the server, channel and person

1. In the main window click **+ New binding**, type a name (e.g. `Ping friend`).
2. **Action** → **Discord Ping**.
3. **Server ID** → **Load ▾** and pick your server (or paste its ID).
4. **Channel ID** → **Load ▾** and pick a text channel.
5. **Target user ID** – paste the person's ID, then click **Check** to see their
   name. To copy an ID in Discord: **User Settings → Advanced → Developer
   Mode** on, then right-click the person → **Copy User ID**. (Right-click a
   server or channel → **Copy Server ID / Copy Channel ID** works the same way.)

   The person must be able to see the channel, otherwise Discord does not
   notify them.

## 6. Set the hotkey

1. In **Shortcut**, choose the type:
   - **Standard** – a key with optional modifiers: `F8`, `Ctrl + Q`, `Alt + F`.
   - **Simultaneous** – several keys held together: `A + S + D`.
   - **Sequence** – keys one after another: `Q → Q → E`.
2. Click **Record** and press the keys. Standard: press the shortcut (Esc
   cancels). Simultaneous: hold all keys, then release. Sequence: press the
   keys one by one, then click **Stop**.
3. Read the line under the shortcut: it warns about conflicts with other
   bindings. **Clear** removes the shortcut (the binding can still be run
   with the big button or from the tray).
4. Click **Save**. If Windows refuses the shortcut because another app already
   uses it, NoKey tells you immediately – pick another one.

Test it: select the binding and press the big **PING** button, or press the
hotkey while another app is focused. The status line shows the result.

## 7. Start with Windows

Settings → **General**:

- **Start with Windows** – adds NoKey to your user's startup list (no admin
  rights). It then starts in the tray.
- **Minimize to system tray** – closing/minimizing hides the window; hotkeys
  keep working. Exit from the tray menu.
- **Launch minimized** – start in the tray even when opened manually.
- **Show tray notifications** – show the result of hotkey actions while the
  window is hidden.
- **Theme** and **Language** – applied at once.
- **Cooldown** – minimum time between two runs of the same binding
  (0.01–3600 s; each binding can also have its own on its Trigger tab).

**Ctrl + Alt + Pause** (or the big **DISABLE ALL BINDS** button) turns all
bindings off and back on — handy for games. It always works and can't be
assigned to a binding.

Click **Save**.

## Troubleshooting

| Message / problem | Cause and fix |
|-------------------|---------------|
| *Bot token is invalid or was reset* | The token was mistyped or reset in the portal. Copy a fresh token (Reset Token) and Connect again. |
| *The bot can't see this channel* | The bot isn't in the server or has no View Channel permission there. Re-invite it (step 3) or fix the channel permissions. |
| *The bot lacks the Send Messages permission* | Allow Send Messages for the bot's role in that channel. |
| *Channel not found* / *User not found* | Wrong ID. Copy it again with Developer Mode. Use a **text** channel. |
| *Discord rate limit reached* | Too many messages in a short time. Wait the shown number of seconds. |
| *Network: …* | No internet, a firewall, or a proxy problem. NoKey uses the Windows proxy settings. |
| *… is already used by Windows or another app* | Another program registered that shortcut. Choose another one or close that program. |
| Simultaneous / sequence shortcut does nothing in one app | That app runs as administrator; Windows hides its keys from normal apps. Run NoKey as administrator too, or use a Standard shortcut. |
| *Cooldown, wait …* | The same binding ran less than the cooldown ago. Change it in Settings. |
| Mention arrives but the person gets no notification | They muted the server/channel or can't see the channel. |
| *Settings file is corrupted* | NoKey kept the old file as `config.corrupt-*.json` in `%APPDATA%\NoKey` and started with defaults. |
| Nothing happens at all | Check the tray: bindings may be paused (Ctrl + Alt + Pause). |
