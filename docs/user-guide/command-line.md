# Copying a host as a command line

A saved host can travel as one command line: to another machine, into a setup script, or to a colleague. The command carries every setting of the host. It never carries a password.

## Copy a host

Right-click the host in the sidebar (or press Shift+F10) and choose **Copy as command line**. On Windows the command is quoted for PowerShell; on Linux and macOS for a POSIX shell such as bash or zsh:

```powershell
1-1.exe import --name 'Build box' --host 10.0.0.51 --port 3389 --username dev `
  --display-mode fit --resize-mode dynamic --scaling automatic --scale-percent 100 `
  --color-depth 32 --bulk-compression off --clipboard-direction bidirectional `
  --save-password true … --add-to-lib Lab --fingerprint SHA256:3q2+7w…
```

(Shown wrapped here; the copied command is one line.)

- Each setting is one flag, named after the setting (`--full-window-drag`, `--bottom-bar`, …).
- The keyboard settings are `--keyboard-layout` (`automatic`, `serverDefault`, or a Windows layout ID as eight hex digits, for example `00000407` for German), `--keyboard-keys` (`automatic` or `physical`) and `--super-as-ctrl` (`true` or `false`).
- `--security` is `automatic` (TLS, with NLA where offered) or `legacy` for hosts that accept only Standard RDP Security ([what that costs](legacy-security.md)).
- `--add-to-lib` names the library the host belongs to.
- `--fingerprint` is included when the host's server key has been confirmed (see [server identity](server-identity.md)). Without a confirmed key it is left out.

On Windows the program is called as `1-1.exe`, because PowerShell reads a bare `1-1` as arithmetic. Run it from 1-1's installation folder, or add that folder to your `PATH`.

## Import a host

Run the command where 1-1 is installed. 1-1 opens (or, if it is already running, comes to the front) and shows **Import …?** with the address, user and library. Nothing is saved until you choose:

- **Import** saves the host into the library named by `--add-to-lib`, which is created if it does not exist yet. Without `--add-to-lib` the host goes into the first library.
- **Import and connect** saves it and connects. 1-1 asks for the password, since the command carries none.

If the command has a fingerprint, or you paste one into the dialog, 1-1 first reads the server's key without credentials and compares it:

- If they match, the key is confirmed, exactly as with **Review server identity**.
- If they differ, the host is saved but its key is not confirmed and 1-1 does not connect. Check with the server's administrator before trusting it.

A command with an unknown option, a mistyped value or no `--name` or `--host` is refused as a whole, so a half-configured host is never saved.
