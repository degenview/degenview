# Bundled fonts

`NotoSansSymbols2-subset.ttf` is **Noto Sans Symbols 2** (version 2.008, © The Noto Project
Authors), licensed under the SIL Open Font License 1.1 (`OFL.txt`). It has no Reserved Font Name.

It is the last glyph fallback in both font families (`ui::theme::install_fonts`), so symbols that
neither egui's bundled fonts nor the macOS system fonts have (▲ ▼ ● ■ ◆ ★ ✔ ✖ ⚠ …, as in a Pine
`plotchar`, a label or a text drawing) draw instead of showing tofu squares, on every platform.

It is a subset of the original, kept to the blocks scripts use: Arrows, Miscellaneous Technical,
Geometric Shapes, Miscellaneous Symbols, Dingbats and Miscellaneous Symbols and Arrows (93 KB of the
original's 1.2 MB). To regenerate it:

```sh
curl -fsSLO https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf
pyftsubset NotoSansSymbols2-Regular.ttf \
  --unicodes="U+2190-21FF,U+2300-23FF,U+25A0-25FF,U+2600-26FF,U+2700-27BF,U+2B00-2BFF" \
  --layout-features='' --no-hinting --output-file=NotoSansSymbols2-subset.ttf
```

(`pyftsubset` is from `pip install fonttools`.) Not covered: the flags ⚐ ⚑, which are in Noto Sans
Symbols, not Symbols 2.
