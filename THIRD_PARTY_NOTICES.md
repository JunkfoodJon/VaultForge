# Third-party notices

VaultForge's own code and art are covered by [LICENSE.md](LICENSE.md). This page lists
everything else it contains, and whose it is.

## Fonts bundled by egui

The interface uses the default fonts that ship inside the `epaint` crate. They are
embedded in the program:

| Font | License | Text |
|---|---|---|
| Hack | MIT, with the Bitstream Vera license | [licenses/egui-fonts/Hack-Regular.txt](licenses/egui-fonts/Hack-Regular.txt) |
| Ubuntu Light | Ubuntu Font Licence 1.0 | [licenses/egui-fonts/UFL.txt](licenses/egui-fonts/UFL.txt) |
| Noto Emoji | SIL Open Font License 1.1 | [licenses/egui-fonts/OFL.txt](licenses/egui-fonts/OFL.txt) |
| emoji-icon-font | MIT | [licenses/egui-fonts/emoji-icon-font-mit-license.txt](licenses/egui-fonts/emoji-icon-font-mit-license.txt) |

Anyone redistributing a built `VaultForge.exe` should include these license texts. The
release downloads already do.

## Rust crates

The program is built from about 200 open-source crates, listed with exact versions in
`Cargo.lock`. They are all under permissive licenses: MIT, Apache-2.0, BSD-2/3-Clause,
ISC, Zlib, Unicode-3.0, BSL-1.0, 0BSD, Unlicense or CC0. None is under the GPL, LGPL,
AGPL or MPL. The direct dependencies are:

- `eframe` / `egui` (MIT OR Apache-2.0), including the `glow` and `wgpu` renderers
- `rfd` (MIT), for the folder picker
- `serde` and `serde_json` (MIT OR Apache-2.0)
- `walkdir` (Unlicense OR MIT)
- `embed-resource` (MIT) and `image` (MIT OR Apache-2.0), used only at build time

For the full license text of every crate, run a tool such as `cargo about` or
`cargo license` in this folder.

## Installer

The setup program is built with [NSIS](https://nsis.sourceforge.io) (zlib/libpng
license). NSIS's license lets installers be distributed freely.

## Names

Windows, Rust, Cargo, Python and other product names are trademarks of their owners.
They are used only to describe what VaultForge works with. VaultForge is not affiliated
with or endorsed by any of them.
