# Localization

How the interface is shown in more than one language. The runtime is `src/i18n/`, a leaf module like
`format.rs` and `platform.rs`: every layer may import it and it imports none of them.

The pieces are the standard ones, so a translator's tools and a new engineer's expectations work:

| Need | Standard used |
| --- | --- |
| Catalogs | GNU gettext `.po` files and a `.pot` template (`polib` reads them) |
| Plural forms | the catalog's own `Plural-Forms` header, tested against CLDR (`icu_plurals`) |
| Language tags and fallback | BCP 47 and CLDR's fallback chain (`icu_locale_core`, `icu_locale`) |
| The system's languages | `sys-locale` |
| Dates and times | CLDR patterns through `icu_datetime` |

**State.** The runtime, the language setting and Settings ▸ Appearance ▸ Language exist, and the
shared date helpers (`ui::kit::time_text`, `i18n::datetime`, Settings ▸ Appearance) are translated.
**No language but English ships**, so the language card is hidden in release builds. The rest of the
interface is still English literals; moving it over, directory by directory, is the next step.

## Writing text

User-facing text is written in English where it is used, through one of three macros that return it in
the language in use. The English text is the key of the catalogs (gettext's `msgid`).

| Write | Returns | For |
| --- | --- | --- |
| `tr!("Save")` | `&'static str` | A label. Goes wherever a literal went. |
| `tr!("{symbol} crossed {price}", symbol = s, price = p)` | `String` | A sentence with values, put in by name. |
| `tr_n!(count, "{count} alert", "{count} alerts")` | `String` | A counted message: the plural form the language uses for `count`. `{count}` is passed for you. |
| `tr_c!("month abbreviation", "Jan")` | `&'static str` | English that translates differently in different places (the same word as a verb and as a noun). The context is the first argument (`msgctxt`). |

The compiler checks each message (`i18n::check`, a `const` block in the macros), so a mistake is a
build error at the call:

- The text is a **plain string literal**: not a `const`, a `format!` or a `concat!`.
- Every `{name}` in the text is passed as `name = value`, and every value passed is in the text (a
  counted message may leave `{count}` out of one form). A message with no values has no braces.
- No leading or trailing space in the text, and not empty.

Rules that need a person:

- **Never build a sentence from pieces.** `format!("{} {}", tr!("Delete"), name)` cannot be
  translated: another language puts the words in another order. Write one message with the value in it.
- Never `if count == 1 { "item" } else { "items" }`; that is `tr_n!`.
- **One message, one meaning.** Reuse a text only for the same meaning; use `tr_c!` when two places
  need different translations of the same English (a test fails on a text written both counted and
  plain).
- **Do not keep the text.** A language chosen in Settings changes what the macros return, so a
  `&'static str` or `String` from one belongs to the frame being drawn. A cache that must survive a
  frame is made again when `i18n::revision()` changes. (A `const` or `static` cannot hold one: the
  macros are not `const`.)
- Do not translate what is not interface text: symbols, tickers, Pine source and its errors, log
  lines, MCP tool descriptions, market and provider data, file formats, keys of saved data.
- Wording that depends on the platform (`platform.rs`) is converted with the rest of the interface;
  until then it stays English in every language.

A widget's `egui::Id` must not be made from translated text: the id would change with the language.
Make it from the enum variant or a fixed name.

### Dates and times

Call `i18n::datetime` (`date`, `month_day`, `month_day_time`, `weekday_month_day`, `month_year`,
`weekday_name`, `weekday_short`, …); never a `MONTHS` table of your own and never a `tr!` that puts a
month name in a sentence. The language's own order, names and separators come from CLDR.

- English output is what it always was (`Oct 9, 2026`, `Oct 9, 14:03`, `Monday, October 9`), except
  Settings ▸ About, which wrote the build date day first (`11 Oct 2026`) and now writes it like every
  other date (`Oct 11, 2026`).
- Times are on a 24-hour clock in every language, as a trading app shows them. `month_day_clock` is the
  one place that follows the language's own clock (the alert drawer).
- ICU writes U+202F before "PM"; it is turned into a plain space, because fonts rarely have it.
- Weekday initials are the first letter of the language's short name, capitalised.
- Relative time ("5 min. ago") stays `tr_n!` messages: ICU4X has no stable formatter for it.

## Languages

`i18n/locale.rs` has one table, `LANGUAGES`. `English` is the source. `Pseudo` (`en-XA`) is a tool:
accented, about 40% longer and in brackets, so text that was never put through `tr!` shows without
brackets, a label that is cut shows its missing `]`, and a fixed width that is too small shows at once.
It is offered in the language picker of debug builds only.

- **Look at a screen in it:** `DEGENVIEW_LOCALE=en-XA cargo run`, or pick it in Settings ▸ Appearance
  (debug builds). `DEGENVIEW_LOCALE` names the language for one run over the saved choice and the
  system's; use it for screenshots too.
- **In a test:** `i18n::with_locale(Locale::PSEUDO, || …)` sets the language for the test's own
  thread, so tests running beside it are not affected. Never call `i18n::set` in a test: it changes the
  language of every thread.

### Choosing the language

`DEGENVIEW_LOCALE` (a tag) over the saved choice (`Preference::Fixed`) over the system's first language
the app has (`Locale::negotiate`: exact tag, then CLDR's fallback chain, so `es-MX` gets Spanish and
`zh-Hant-TW` gets Traditional Chinese) over English. The choice is the `language` setting (`"system"` or
a tag), local to the Mac: it is not in `sync::record::policy`, so another Mac is not moved.
`host::language` reads it at startup and applies a change from Settings; `app/tabs.rs` repaints every
window when `i18n::revision()` changes, because a window with nothing animating would keep the old
words.

The system's languages come from `service::system::language`, which asks `sys-locale`.

## Catalogs

One gettext file per language, `assets/locales/<tag>.po`, compiled in (`LANGUAGES`).
`assets/locales/messages.pot` is the template: every message of the source, untranslated, with the
files that write it. A test fails when the template lacks a message the source writes or lists one it no
longer does, and names them. That is the whole contract: the order, the header and the `#:` references
are informational (`msgmerge` and translation platforms refresh them), so moving a message to another
file or releasing a version does not fail it. Rewrite the file with

```bash
UPDATE_POT=1 cargo test --lib i18n::tests::the_template_lists
```

```po
msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\n"
"Language: es\n"
"Plural-Forms: nplurals=2; plural=(n != 1);\n"

msgid "Save"
msgstr "Guardar"

msgctxt "month"
msgid "May"
msgstr "mayo"

msgid "{count} alert"
msgid_plural "{count} alerts"
msgstr[0] "{count} alerta"
msgstr[1] "{count} alertas"
```

- A message is found by its English text (`msgid`) and its context (`msgctxt`); a counted one by its
  singular, with one `msgstr[i]` per form of the header's `Plural-Forms`.
- A message the catalog lacks, leaves empty, or marks `fuzzy` shows in English. Nothing breaks while a
  language is incomplete.
- Bring a catalog up to date with the source: `msgmerge -U assets/locales/es.po
  assets/locales/messages.pot` (a translation platform such as Weblate does it by itself).
- `every_catalog_matches_the_messages_in_the_source` fails on a message that is no longer in the
  source (stale), a `{name}` the source does not pass, a text that uses other values than its English,
  and a message counted in one place and not in the other. A catalog whose `Plural-Forms` disagrees
  with CLDR for its language is caught by `the_plural_formulas_translators_write_agree_with_cldr`,
  once the language's formula is in it.

### Adding a language

1. Add a row to `LANGUAGES` (`tag`, `native_name`, `Kind::Natural`, and its `include_str!` of the
   catalog). Plural rules and date patterns come from the catalog's header and CLDR; no code per
   language.
2. `msginit` or the translation platform makes `assets/locales/<tag>.po` from `messages.pot`. Its
   `Plural-Forms` is the standard GNU one for the language.
3. Add the language's formula to `the_plural_formulas_translators_write_agree_with_cldr`.
4. **Fonts.** egui draws from the fonts it is given and has no system fallback. Latin, Cyrillic and
   Greek are in the macOS system font and egui's own; Chinese, Japanese, Korean, Thai, Arabic, Hebrew
   and Indic scripts need a font added to `ui::theme::base_fonts` (a system font on macOS, a bundled
   or found one elsewhere; Han characters need the font order set per language).
5. Draw Settings and a chart screen in the language and in the pseudo-locale at the smallest window
   size, in both themes, and look at the pictures.
6. Add the language to the packaging that names languages (`CFBundleLocalizations`, the installer's
   `[Languages]`, the Flatpak metainfo) once those carry translations.

## Not done yet

- **Right-to-left languages.** egui's text layout does not apply the bidirectional algorithm
  (`epaint` `TODO(emilk): heed bidi characters`) and the layouts are not mirrored. Arabic, Hebrew and
  Persian wait on that.
- **IME.** The Pine editor handles keys itself and no `Event::Ime` is read; composing Chinese,
  Japanese or Korean there is unchecked. Plain `TextEdit` fields use egui's.
- **Numbers.** Prices and chart axes keep one format (`format.rs`, `model/chart/price_format.rs`);
  decimal and thousands separators (`icu_decimal`) are a decision to make with the first language that
  differs.
- **First day of the week.** The date pickers start on Monday. CLDR knows each region's first day
  (`icu_calendar`'s week information); the pickers' grids need to take it.
- **The macOS menu bar and notifications.** `native_bridge.rs` builds the menus with English titles,
  and the alert agent is another process that sends notification text without `tr!`. Both are to
  be converted, and the agent needs the language setting read from the store.
- **The language of the AI assistant** and of the docs site (its own repository).
- **A bare-literal lint** like the layering test's debt lists, once most of the interface is converted:
  it would fail a literal in a widget call outside a list of files that still have them.
