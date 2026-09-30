# Contributing

Thanks for your interest in VaultForge. It's a one-person project, so this page keeps
things simple.

## Bugs and ideas

Open an [issue](https://github.com/JunkfoodJon/VaultForge/issues/new/choose) and pick the
bug report or feature request form. For anything security-related, please follow
[SECURITY.md](SECURITY.md) instead of opening a public issue.

## Pull requests

Small, focused pull requests are welcome. For anything larger, please open an issue
first so we can agree on the approach before you spend time on it.

Before you open one:

- build with `cargo build --release --locked` and run `cargo test`;
- if you changed the UI, include a screenshot in the pull request;
- keep the app light: no background polling while idle, and no new dependency without a
  good reason;
- start other programs with an argument list, never through a shell;
- match the style of the code around your change. Colours come from `src/theme.rs`,
  which VaultLauncher shares.

## License of contributions

VaultForge is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE.md). By
opening a pull request you agree that your contribution is released under the same
license, and that the maintainer may include it in VaultForge under that license.
