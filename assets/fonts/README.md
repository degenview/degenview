# Bundled fonts

Two fonts are compiled into DegenView (`include_bytes!` in `src/ui/theme.rs`), both under the
**SIL Open Font License 1.1** and neither with a Reserved Font Name. The licence texts are
`OFL-JetBrainsMono.txt` and `OFL-NotoSansSymbols2.txt`. They are copied into the macOS app
(`Contents/Resources/Licenses/`), the Flatpak (`/app/share/licenses/es.nico.degenview/`) and the Linux
and Windows downloads, because the licence requires its text and the copyright notice to accompany
every copy of the font, a program that embeds it included. DegenView itself is GPL-3.0-only; the fonts keep
their own licence.

| File | Font | Used for | Licence file |
| --- | --- | --- | --- |
| `JetBrainsMonoNL-Regular.ttf` | JetBrains Mono NL Regular 2.304 | the Pine editor's code (the default font), and the first stop of the code families | `OFL-JetBrainsMono.txt` |
| `NotoSansSymbols2-subset.ttf` | Noto Sans Symbols 2 2.008, subset | the last glyph fallback of every font family | `OFL-NotoSansSymbols2.txt` |

## JetBrains Mono

**JetBrains Mono** 2.304 (2023-01-14), © 2020 The JetBrains Mono Project Authors
(<https://github.com/JetBrains/JetBrainsMono>), licensed under the SIL Open Font License 1.1
(`OFL-JetBrainsMono.txt`, byte for byte the upstream `OFL.txt` of that release). It has no Reserved Font Name.
Typeface by JetBrains; <https://www.jetbrains.com/lp/mono/>.

It is the default font of the Pine editor (Settings ▸ Script Manager ▸ Typography). Only the **NL**
(no ligatures) **Regular** build is bundled, and the file is **unmodified**: the same bytes as the
release's `fonts/ttf/JetBrainsMonoNL-Regular.ttf` (208,576 bytes). NL has the same glyphs and metrics as the
regular build, minus the code ligatures: DegenView draws each syntax colour as its own run of text and cannot
switch ligatures on or off, so ligatures would form in some runs and not in others. The editor draws one
style, so Bold and Italic are not bundled.

```
sha256  fb3b2575d7b0657359707993288f12a7360344d39387bb26050e276d61f6bd2a  JetBrainsMonoNL-Regular.ttf
```

A test (`bundled_fonts_tests`) checks that hash and that both licence texts and both READMEs are in place. To
fetch it again, or to check it:

```sh
curl -fsSLO https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip
unzip -j JetBrainsMono-2.304.zip fonts/ttf/JetBrainsMonoNL-Regular.ttf OFL.txt
shasum -a 256 JetBrainsMonoNL-Regular.ttf        # the hash above
cmp OFL.txt OFL-JetBrainsMono.txt                 # no output: the same text
```

A font installed on the computer that is also called "JetBrains Mono" is not used instead: the built-in one
is, so the editor looks the same on every machine.

## Noto Sans Symbols 2

`NotoSansSymbols2-subset.ttf` is **Noto Sans Symbols 2** (version 2.008, © 2022 The Noto Project Authors,
<https://github.com/notofonts/symbols>), licensed under the SIL Open Font License 1.1
(`OFL-NotoSansSymbols2.txt`). It has no Reserved Font Name.

It is the last glyph fallback in both font families (`ui::theme::install_fonts`), so symbols that
neither egui's bundled fonts nor the macOS system fonts have (▲ ▼ ● ■ ◆ ★ ✔ ✖ ⚠ …, as in a Pine
`plotchar`, a label or a text drawing) draw instead of showing tofu squares, on every platform.

It is a subset of the original, kept to the blocks scripts use: Arrows, Miscellaneous Technical,
Geometric Shapes, Miscellaneous Symbols, Dingbats and Miscellaneous Symbols and Arrows (93 KB of the
original's 1.2 MB). The OFL allows a modified version when it carries no Reserved Font Name, as here. To
regenerate it:

```sh
curl -fsSLO https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf
pyftsubset NotoSansSymbols2-Regular.ttf \
  --unicodes="U+2190-21FF,U+2300-23FF,U+25A0-25FF,U+2600-26FF,U+2700-27BF,U+2B00-2BFF" \
  --layout-features='' --no-hinting --output-file=NotoSansSymbols2-subset.ttf
```

(`pyftsubset` is from `pip install fonttools`.) Not covered: the flags ⚐ ⚑, which are in Noto Sans
Symbols, not Symbols 2.
