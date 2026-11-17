# Localization

How the interface is shown in more than one language. The runtime is `src/i18n/`, a leaf module like
`format.rs` and `platform.rs`: every layer may import it and it imports none of them.

**State.** The runtime, the language setting and Settings ▸ Appearance ▸ Language exist, and the
shared date helpers (`ui::kit::time_text`, `i18n::calendar`, Settings ▸ Appearance) are translated.
**No language but English ships**, so the language card is hidden in release builds. The rest of the
interface is still English literals; moving it over, directory by directory, is the next step.

## Writing text

User-facing text is written in English where it is used, through one of four macros that return it in
the language in use. The English text is the key of the catalogs.

| Write | Returns | For |
| --- | --- | --- |
| `tr!("Save")` | `&'static str` | A label. Goes wherever a literal went. |
| `tr!("{symbol} crossed {price}", symbol = s, price = p)` | `String` | A sentence with values, put in by name. |
| `tr_n!(count, "{count} alert", "{count} alerts")` | `String` | A counted message: the plural form the language uses for `count`. `{count}` is passed for you. |
| `tr_c!("month abbreviation", "Jan")` | `&'static str` | English that translates differently in different places (the same word as a verb and as a noun). The context is the first argument. |

Rules the tests enforce (`i18n/tests.rs`):

- The text is a **plain string literal**: not a `const`, a `format!` or a `concat!`.
- Every `{name}` in the text is passed as `name = value`, and every value passed is in the text (a
  counted message may leave `{count}` out of one form).
- No leading or trailing space in the text, and not empty.
- **Never build a sentence from pieces.** `format!("{} {}", tr!("Delete"), name)` cannot be
  translated: another language puts the words in another order. Write one message with the value in it.
- Never `if count == 1 { "item" } else { "items" }`; that is `tr_n!`.
- **One message, one meaning.** Reuse a text only for the same meaning; use `tr_c!` when two places
  need different translations of the same English.
- Do not translate what is not interface text: symbols, tickers, Pine source and its errors, log
  lines, MCP tool descriptions, market and provider data, file formats, keys of saved data.
- Wording that depends on the platform (`platform.rs`) is one whole message per platform
  (`if cfg!(windows) { tr!(…) } else { tr!(…) }`), never a platform word put into a sentence.
  `platform::computer()` and `file_manager()` still return a word ("Mac", "Finder"), translated with a
  context, for `tr!("… {computer} …", computer = …)` and `"Show in {…}"`. A language that inflects
  the word cannot use them; the fix is a whole message per platform in `platform.rs` and the caller
  switched to it, as the credential hints are.

A widget's `egui::Id` must not be made from translated text: the id would change with the language.
Make it from the enum variant or a fixed name.

`ui::kit::time_text` and `i18n::calendar` hold the date and month names; call them instead of a
`MONTHS` table of your own.

## Languages

`Locale` lists them. `English` is the source. `Pseudo` (`en-XA`) is a tool: accented, about 40% longer
and in brackets, so text that was never put through `tr!` shows without brackets, a label that is cut
shows its missing `]`, and a fixed width that is too small shows at once. It is offered in the language
picker of debug builds only.

- **Look at a screen in it:** `DEGENVIEW_LOCALE=en-XA cargo run`, or pick it in Settings ▸ Appearance
  (debug builds). `DEGENVIEW_LOCALE` names the language for one run over the saved choice and the
  system's; use it for screenshots too.
- **In a test:** `i18n::with_locale(Locale::Pseudo, || …)` sets the language for the test's own
  thread, so tests running beside it are not affected. Never call `i18n::set` in a test: it changes the
  language of every thread.

### Choosing the language

`DEGENVIEW_LOCALE` (a tag) over the saved choice (`Preference::Fixed`) over the system's first language
the app has (`Locale::negotiate`: exact tag, then language, so `es-MX` gets Spanish) over English.
The choice is the `language` setting (`"system"` or a tag), local to the Mac: it is not in
`sync::record::policy`, so another Mac is not moved. `host::language` reads it at startup and applies a
change from Settings; `app/tabs.rs` repaints every window when `i18n::revision()` changes, because a
window with nothing animating would keep the old words.

The system's languages come from `service::system::language`: `NSLocale.preferredLanguages` on macOS,
`GetUserPreferredUILanguages` on Windows, `LANGUAGE` / `LC_ALL` / `LC_MESSAGES` / `LANG` elsewhere.

## Catalogs

One JSON file per language, `assets/locales/<tag>.json`, compiled in (`i18n::catalog::source`).

```json
{
  "messages": {
    "Save": "Guardar",
    "{count} alerts": { "one": "{count} alerta", "other": "{count} alertas" }
  },
  "contexts": { "month abbreviation": { "Jan": "ene" } }
}
```

- A message is found by its English text; a counted one by its `other` text.
- A counted message gives its forms by CLDR category (`zero`, `one`, `two`, `few`, `many`, `other`);
  `other` is required. A language with one form (Japanese) gives a plain string.
- A message the catalog lacks shows in English. Nothing breaks while a language is incomplete.
- `every_catalog_matches_the_messages_in_the_source` fails on a message that is no longer in the
  source (stale), a `{name}` the source does not pass, a text that uses other values than its English,
  and a message counted in one place and not in the other.

### Adding a language

1. Add the variant to `Locale` (`ALL`, `tag`, `language`, `native_name`), and its plural rule to
   `Locale::plural_category` with tests for the counts that separate its forms (CLDR's rules).
2. Add `assets/locales/<tag>.json` and its `include_str!` arm in `catalog::source`.
3. **Fonts.** egui draws from the fonts it is given and has no system fallback. Latin, Cyrillic and
   Greek are in the macOS system font and egui's own; Chinese, Japanese, Korean, Thai, Arabic, Hebrew
   and Indic scripts need a font added to `ui::theme::base_fonts` (a system font on macOS, a bundled
   or found one elsewhere; Han characters need the font order set per language).
4. Draw Settings and a chart screen in the language and in the pseudo-locale at the smallest window
   size, in both themes, and look at the pictures.
5. Add the language to the packaging that names languages (below) once those carry translations.

### Packaging that names languages

None of it names a language today, on purpose: with English the only one, a list of one tells the
system nothing and a translated name would be a lie. When the first translation ships, each needs:

- **macOS bundle** (`tools/bundle.sh` writes both `Info.plist` files). Add `CFBundleDevelopmentRegion`
  (`en`) and `CFBundleLocalizations` (`en` plus each tag) to the app's and to the alert agent's
  `Info.plist`. Without the list macOS treats the app as English-only: it keeps the standard strings
  it draws itself (the Services and Window menus' own items, the open and save panels, the
  notification permission prompt, the Dock's menu) in English whatever the system language. The
  menu items the app titles (`native_bridge.rs`) and the notification text follow the app's own
  setting either way. `NSLocale.preferredLanguages`, which `service::system::language` reads, is the
  user's list whatever the bundle declares, so it needs no change. Check the `.app` in `System
  Settings ▸ Language & Region ▸ Applications` and the Dock menu in the new language.
- **Windows installer** (`packaging/windows/degenview.iss`). It has no `[Languages]` section, so
  Inno Setup's wizard is English. Add `Name: "en"; MessagesFile: "compiler:Default.isl"` first, then
  `Name: "<tag>"; MessagesFile: "compiler:Languages\<Name>.isl"` for each, and the wizard follows the
  system language. A language Inno Setup ships no `.isl` for needs one added under `packaging/windows/`.
  The installer's own `[CustomMessages]` and the shortcut names (`[Icons]`) are English text to
  translate there. The app inside does not read the installer's choice; it follows the setting.
- **Flatpak metainfo** (`packaging/flatpak/es.nico.degenview.metainfo.xml`). Add the translated
  `<summary>` and `<description>` as repeated elements with `xml:lang="<tag>"` after the untagged
  English ones, and the same for `Name` / `Comment` in `es.nico.degenview.desktop` (`Comment[es]=…`).
  Release notes in `<release>` stay English unless they are written per language. Flathub's linter
  reads these; run `flatpak run org.freedesktop.appstream-glib validate` on the file and check that
  the sandbox passes the locale (`LANG`) the app's language reader looks at (`docs/flatpak.md`).
- **Docs and the README** have their own languages; see "Not done yet".

## Not done yet

- **Right-to-left languages.** egui's text layout does not apply the bidirectional algorithm
  (`epaint` `TODO(emilk): heed bidi characters`) and the layouts are not mirrored. Arabic, Hebrew and
  Persian wait on that.
- **IME.** The Pine editor handles keys itself and no `Event::Ime` is read; composing Chinese,
  Japanese or Korean there is unchecked. Plain `TextEdit` fields use egui's.
- **Numbers.** Prices and chart axes keep one format (`format.rs`, `model/chart/price_format.rs`);
  localised digits and separators are a decision to make with the first language that differs.
- **The macOS menu bar and notifications** are converted (`native_bridge.rs` retitles the menus when
  `i18n::revision()` changes; the alert agent reads the language setting through
  `host::language::Follower`). What is left in them:
  - Items AppKit adds by itself (the Window menu's list of windows, Help's search field, Services'
    entries) are in the system's language, not the app's, until the bundle declares its languages
    (see "Packaging that names languages").
  - The language on a machine whose system language changes while the app or the agent runs is read
    again only at the next start; a choice made in Settings is followed at once.
  - A delivery failure is stored as English text in the alert history (`delivery.error`): it is
    written once, by whichever process tried, and not translated.
- **The language of the AI assistant** and of the docs site (its own repository).
- **A bare-literal lint** like the layering test's debt lists, once most of the interface is converted:
  it would fail a literal in a widget call outside a list of files that still have them.
