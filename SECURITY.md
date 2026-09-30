# Security policy

VaultForge starts other programs (cargo, Python and your own projects), runs a
background hotkey listener, and can stop processes when you ask it to. Reports about any
of that are welcome and taken seriously.

## Reporting a vulnerability

Please **don't open a public issue** for a security problem. Use GitHub's private
reporting: on this repository's **Security** tab, choose **Report a vulnerability**.
Only the maintainer can see the report.

Please include:
- what an attacker can do, and what they need first (a crafted project folder? a file in
  `%APPDATA%\VaultTools`? a local account?);
- steps or a proof of concept;
- the version or commit you tested.

You'll get an acknowledgement as soon as the maintainer can manage. This is a free,
one-person project, so there is no bug bounty. Once a fix ships, reporters are credited
unless they'd rather not be.

## In scope

- Command or argument injection: every command is started with an argument list, never
  through a shell, and anything that breaks that is a bug. The one PowerShell call (the
  portable version's Start Menu shortcut) quotes every value it embeds.
- VaultForge running something the user didn't choose, or running it from somewhere
  other than the project folder shown.
- Stopping a process that isn't running from the project's own `target` folder, or
  stopping one without the user's click.
- The hotkey listener launching an executable other than VaultForge, or being made to.
- Any network traffic started by VaultForge itself (there should be none; see
  [PRIVACY.md](PRIVACY.md)).

## Out of scope

- Code in a project you chose to build or run. `cargo build` runs build scripts and
  procedural macros, and Run and Test run the program itself, exactly as they would from
  a terminal. Only build projects you trust.
- An attacker who can already write to your user profile. They can change what any of
  your programs does.

## Verifying a download

Every release lists SHA-256 checksums in `SHA256SUMS.txt`. Check a download in PowerShell:

```powershell
Get-FileHash .\VaultForge-0.1.0-setup.exe -Algorithm SHA256
```

The result must match the line for that file. The releases are not code-signed yet, so
Windows SmartScreen will say the publisher is unknown. If you'd rather not trust a prebuilt
file, build it yourself; see the README.
