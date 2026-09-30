# Privacy

VaultForge has no account, no telemetry, no analytics, no crash upload and no update
check. **VaultForge itself makes no network connections at all.**

## What goes over the network

Only what the tools you start do. When you build a Rust project, `cargo` may download
crates from crates.io (or whatever registry the project uses). When you run a project,
that program does whatever it does. VaultForge starts these commands only when you press
**EXECUTE** or Enter, and it shows the exact command line in the terminal first.

## What it reads

- The folder you choose to scan. It lists folder names and file names and reads file
  modification times; it doesn't read or upload file contents. To find a project's entry
  point it checks whether files such as `src\main.rs` or `main.py` exist.
- The output of the commands it runs, to show progress, warnings and errors.
- The list of running processes, and only when you start Build, Run, Test or Clean on a
  Rust project: it looks for programs running from that project's `target` folder, which
  would block the build. Nothing is stopped unless you click **STOP & …**.

## What it stores

All in `%APPDATA%\VaultTools`, on your PC only:

| File | Contents |
|---|---|
| `shared-history.json` | For each project you've used: its path, pin, last build and run times, preferred Python entry file, and learned build durations with warning/error counts. Shared with VaultLauncher. |
| `vaultforge-settings.json` | The folder you scan and whether the hotkey is on |
| `vaultforge-catalog.json` | The project list from the last scan (paths, names, markers, modification times) |
| `vaultforge-broker-status.json`, `vaultforge-gui.pid`, `vaultforge-shortcuts.stamp` | Housekeeping for the hotkey listener: process ids, the exe path, the version |
| `logs\*.log` | The complete output of each run. Build output can include your file paths, so check a log before you share it. |
| `logs\vaultforge-crash.log` | Written only if VaultForge crashes: the error message and where in VaultForge's code it happened |

## What it changes on your PC

- **Sign-in entry** (while the hotkey is on): the value `VaultForge Hotkey Broker` in
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, which starts the background
  hotkey listener. Turning the hotkey off removes it.
- **Start Menu shortcut:** the installer adds `VaultForge`. A portable copy adds
  `VaultForge Portable` the first time it runs. That shortcut is written through
  PowerShell (`WScript.Shell`), the only time VaultForge starts PowerShell.
- **Installer only:** the program files in `%LOCALAPPDATA%\Programs\VaultForge` and its
  uninstall entry under `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\VaultForge`.

Nothing is written outside your own user profile, and VaultForge never asks for
Administrator rights.

## Removing everything

Uninstall from Settings → Apps and answer **Yes** when it asks about your data. To also
remove the shared history, delete `%APPDATA%\VaultTools\shared-history.json` (VaultLauncher
uses that file too). For the portable version: turn the hotkey off, delete the folder,
the `VaultForge Portable` Start Menu shortcut and `%APPDATA%\VaultTools`.
