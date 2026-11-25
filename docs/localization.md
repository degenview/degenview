# Localization

How the interface is shown in more than one language. The runtime is `src/i18n/`, a leaf module like
`format.rs` and `platform.rs`: every layer may import it and it imports none of them.

The pieces are the standard ones, so a translator's tools and a new engineer's expectations work:

| Need | Standard used |
| --- | --- |
| Catalogs | GNU gettext `.po` files and a `.pot` template (`ferrocat-po` reads and writes them; GNU `msgfmt` is the judge of validity) |
| Plural forms | the catalog's own `Plural-Forms` header, tested against CLDR (`icu_plurals`) |
| Language tags and fallback | BCP 47 and CLDR's fallback chain (`icu_locale_core`, `icu_locale`) |
| The system's languages | `sys-locale` |
| Dates and times | CLDR patterns through `icu_datetime` |

**State.** The runtime, the language setting and Settings ▸ Appearance ▸ Language exist, and the
shared date helpers (`ui::kit::time_text`, `i18n::datetime`, Settings ▸ Appearance) are translated.
**Spanish (Spain)** (`assets/locales/es.po`, tag `es`) was the first translation and **Simplified
Chinese** (`assets/locales/zh.po`, tag `zh`) the second, so the language card shows in
release builds too; it hides itself whenever only one language is offered. The rest of the
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
| `tr_noop!("Image hosting")` | `i18n::Marked` | Text in a `static` or `const` table (a registry of providers), where a macro cannot run. The text is in the catalogs; read it with `.text()` where it is drawn. |

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
- **A run of items is `i18n::join_list`, not `join(", ")`.** CLDR's "and" list pattern (`icu_list`)
  writes `a, b, and c` in English, `a, b y c` in Spanish and `a、b和c` in Chinese. Pass the result to
  `tr!` as a value when the list sits in a sentence; end a truncated list with its own item
  (`tr!("{count} more")`) so the "and" lands before it. Two measurements side by side (a count of bars
  and a duration) are not a list: separate them with ` · `.
- **One message, one meaning.** Reuse a text only for the same meaning; use `tr_c!` when two places
  need different translations of the same English (a test fails on a text written both counted and
  plain).
- **A table of text is marked, not translated where it is written.** A `static` registry (the snapshot
  providers) holds `tr_noop!` values and the code that draws it calls `.text()`. A field typed `Marked`
  cannot be drawn by mistake: it is not a `&str`.
- **A record of what happened is data, not a sentence.** The paper journal stores the English sentence
  (for exports and MCP) and a `PaperNote` (what happened); the table writes the sentence from the note in
  the language in use. A line written before notes existed has none and shows its sentence.
- **Do not keep the text.** The language can change under a running process (the alert agent follows
  Settings, a test sets one with `with_locale`), so a `&'static str` or `String` from a macro
  belongs to the frame being drawn. A cache that must survive a
  frame is made again when `i18n::revision()` changes. (A `const` or `static` cannot hold one: the
  macros are not `const`.)
- Do not translate what is not interface text: symbols, tickers, Pine source and its errors, log
  lines, MCP tool descriptions, market and provider data, file formats, keys of saved data, and
  names the person gave (a layout, a watchlist, a script), including the example ones in a picture.
- Wording that depends on the platform (`platform.rs`) is one whole message per platform
  (`if cfg!(windows) { tr!(…) } else { tr!(…) }`), never a platform word put into a sentence: another
  language writes the sentence its own way and inflects the word. A text that needs the platform's
  name gets a function there that returns the whole sentence ("Secret not found in your Keychain",
  "Show in Finder"). Where a longer text holds one, split it into sentences and join them with `i18n::join_sentences`, which
  writes the space the language wants (none after Chinese 。).
- **A status line keeps no text.** A `StatusMessage` that says the same thing whatever happened, such
  as `StatusMessage::saved_securely()`, is stored as how to write it and written when drawn, so a
  language chosen after the save reaches it. `StatusMessage::ok`/`error` with text keep that text as
  written at the event; use `Words::Written` for a fixed message that can outlive a language change.

A widget's `egui::Id` must not be made from translated text: the id would change with the language.
Make it from the enum variant or a fixed name. `kit::field` takes that name as `id_salt`, apart from the
title it shows; focus and a revealed secret live under it.

Widths are measured from the words, never tuned to English. A column that holds a tag or a status
badge is sized with `grid::tags_width` / `grid::badges_width`; a button a row keeps room for, with
`kit::push_button_width` / `sheet_button_width`; a row of parts that may not fit side by side flows
them by their measured widths (the About footer, `footer_rows`).

### Dates and times

Call `i18n::datetime` (`date`, `month_day`, `month_day_time`, `weekday_month_day`, `month_year`,
`weekday_name`, `weekday_short`, …); never a `MONTHS` table of your own and never a `tr!` that puts a
month name in a sentence. The language's own order, names and separators come from CLDR.

- English output is what it always was (`Oct 9, 2026`, `Oct 9, 14:03`, `Monday, October 9`), except
  Settings ▸ About, which wrote the build date day first (`11 Oct 2026`) and now writes it like every
  other date (`Oct 11, 2026`).
- Dates are **Gregorian in every language** (the formatters are built with `-u-ca-gregory`, in one
  place). The model, Pine and the pickers' grids count Gregorian days, and ICU would otherwise write a
  Persian or Thai date in its own calendar ("Bahman 1404") over a Gregorian grid. A language's own
  calendar is decided with the first language that has one, because the grids and the date parser
  have to follow.
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
- **Text must not be cut.** A label that does not fit wraps (`kit::paragraph`) or makes its row
  taller, or, in a miniature, shrinks to fit (`kit::galley_fitted`); `kit::galley` cuts with an
  ellipsis and is for chrome that has a real width limit. `render_tests::Screen::truncated_text`
  reports the text a frame shows cut with an ellipsis, and a page's pseudo-locale test asserts it is
  empty.
- **In a test:** `i18n::with_locale(Locale::PSEUDO, || …)` sets the language for the test's own
  thread, so tests running beside it are not affected. Never call `i18n::set` in a test: it changes the
  language of every thread.

### Choosing the language

`DEGENVIEW_LOCALE` (a tag, which may name any language, the pseudo-locale included) over the saved
choice (`Preference::Fixed`) over the system's first language the app offers (`Locale::negotiate`:
exact tag, then CLDR's fallback chain, so `es-MX` gets Spanish and `zh-Hant-TW` gets Traditional
Chinese) over English. "Offered" is `Locale::selectable()`, the languages the picker lists: natural
languages, plus the pseudo-locale in debug builds. Resolution takes that list as an input, so a choice
the picker cannot show (a pseudo-locale saved by a debug build and read by a release build, whose
picker is hidden) is kept in the store and not applied, and the system's languages cannot select it
either. The choice is the `language` setting (`"system"` or a tag), local to the Mac: it is not in
`sync::record::policy`, so another Mac is not moved. `host::language` reads it at startup and saves a
change from Settings (`host::language::save`); the language in the app does not change while it runs,
because kept text, the native menus and every open window would each switch at a moment of their own.
Picking a language raises a modal, "Restart to change the language?" (`ask_to_restart` in
`ui/settings/window.rs`; nothing is asked when the pick is the language already in use). Restart Now
sends `SettingsAction::Restart` → `AppAction::Restart`: the app quits through the same question as ⌘Q
and `service::update::install::relaunch_running_app` reopens it. Later keeps the choice saved. A quit
that is called off cancels the reopen. `app/tabs.rs` still repaints every window when
`i18n::revision()` changes, which only the alert agent and tests cause.

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
- A catalog may be laid out any way GNU gettext accepts (entries with or without blank lines, comments,
  wrapped strings, CRLF): an entry starts at its `msgctxt` or `msgid`, as in gettext's grammar. The
  loader reads valid catalogs; deciding that one is valid is `msgfmt`'s, which
  `the_catalogs_and_layouts_are_valid_gettext` runs on every shipped file and on each layout the tests
  use (it needs gettext's tools, which CI's Linux image has).
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

Agents: the procedure, the quality bar (context-specific wording, glossary, consistency pass) and the
verification are in [Translation](agent-guides/translation.md). The steps for the code:

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
6. Add the language to the packaging that names languages (below) once those carry translations.

### Packaging that names languages

Spanish and Simplified Chinese are listed in each of these (the macOS bundle, the Windows installer,
the Flatpak metainfo and desktop entry); a language added later is added to each, as follows:

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
  system language. A language Inno Setup ships no `.isl` for needs one added under `packaging/windows/`:
`ChineseSimplified.isl` is a translation of Inno's own `Default.isl` (every key, the `%1` and
`[name]` values unchanged; UTF-8, `LanguageID=$0804`, code page 936), referenced from the script by its
bare file name. Compile it with the pinned Inno Setup and run the setup with `/LANG=zh` to see it.
  The installer's own `[CustomMessages]` and the shortcut names (`[Icons]`) are English text to
  translate there. The app inside does not read the installer's choice; it follows the setting.
- **Flatpak metainfo** (`packaging/flatpak/es.nico.degenview.metainfo.xml`). Add the translated
  `<summary>` and `<description>` as repeated elements with `xml:lang="<tag>"` after the untagged
  English ones, and the same for `Name` / `Comment` in `es.nico.degenview.desktop` (`Comment[es]=…`).
  Release notes in `<release>` stay English unless they are written per language. Flathub's linter
  reads these; run `flatpak run org.freedesktop.appstream-glib validate` on the file and check that
  the sandbox passes the locale (`LANG`) the app's language reader looks at (`docs/flatpak.md`).
- **Docs and the README** have their own languages; see "Not done yet".

### Spanish

`assets/locales/es.po` is written for Spain: informal *tú*, infinitives on buttons and menu items,
Spain's vocabulary (*ordenador*, *Ajustes*, *pulsar*), and sentence case like the English. It is
tagged `es`, not `es-ES`, so every other Spanish region (`es-MX` → `es-419` → `es`) gets it through
CLDR's fallback chain instead of English. Trading terms Spanish-speaking traders say in English
(Long, Short, Stop, Take profit, Webhook, Pine, Footprint) stay English; the vocabulary for the rest
(Gráfico, Lista de seguimiento, Cartera, Tenencias, Posición, Repetición, Diario, …) is kept by reading
the catalog, so a new message uses the word the neighbouring ones do. Numbers keep the shared format
(see "Not done yet").

### Simplified Chinese

`assets/locales/zh.po` is written for mainland China in the register of a Mac or trading app:
informal 你, bare verbs on buttons and menu items, full-width punctuation, one space between Han
characters and Latin letters, digits or `{values}` as Apple writes them, and no 请. The catalog has one
plural form (`nplurals=1`; counts take a measure word such as 个 or 根). CLDR makes Simplified the
default script of Chinese, so the tag is `zh` and `zh-CN`, `zh-SG`, `zh-Hans` and `zh-Hans-CN` reach it
through the fallback chain, while `zh-TW` and `zh-HK` (`zh-Hant` on the chain) stay on English until a
`zh-Hant` catalog exists. The macOS bundle lists it as `zh-Hans`, which is Apple's name for it.
Traders say
Long, Short, Stop and Take profit in Chinese (做多, 做空, 止损, 止盈), so unlike Spanish they are
translated; Webhook, VWAP, TPO, Pine and MCP stay in English. Vocabulary is kept by reading the
catalog.

The font is the system's, found and read by `ui::theme::cjk` only when the language in use is
Chinese (Hiragino Sans GB on macOS, Microsoft YaHei on Windows, and on Linux the first font of
fontconfig's ranking (`fc-match -s`) that declares zh-cn, or another Chinese language, because each
distribution puts Noto Sans CJK, WenQuanYi or Droid under another name and the best match for
`:lang=zh-cn` can be a font with no Han characters; inside the Flatpak sandbox the ranking lists the
host's fonts under `/run/host/fonts`, which the app can read. Checked with the real Flatpak build on
Fedora (Noto Sans CJK) and Kali (Droid Sans Fallback only)). A machine with none of them shows the
missing-glyph box for every Han character; no font for the catalog is bundled because one with its
characters is megabytes.

The language picker lists each language by its own name whatever language is in use, so `简体中文` must
draw in an English interface, which reads no Chinese font. Its four characters are bundled as a 4 KB
subset of Noto Sans SC (`assets/fonts`), after the system's font in the chain. A test draws every
language's own name with the app's fonts.

egui centres each font's line box in the line and a Chinese font's line gap is large, so its
characters sit about 0.23 em above Inter's; the font is shifted by the difference of the two
fonts' own `ascent - height / 2` (read from their tables, `cjk::baseline_offset`), whichever Chinese
font the machine has. Han characters a person types or a provider sends (a watchlist name, a
Chinese market) are drawn only when the interface is Chinese, apart from the four of the picker.

## Not done yet

- **Right-to-left languages.** egui's text layout does not apply the bidirectional algorithm
  (`epaint` `TODO(emilk): heed bidi characters`) and the layouts are not mirrored. Arabic, Hebrew and
  Persian wait on that.
- **IME.** Pinyin composition on macOS works in the symbol search (a plain `TextEdit`) and in the Pine
  editor: the preedit is drawn underlined and the committed text arrives. Windows, Linux and the other
  input methods (Japanese, Korean) are unchecked.
- **Traditional Chinese** (`zh-Hant`), and Han text under an English interface (see "Simplified Chinese").
- **Numbers.** Prices and chart axes keep one format (`format.rs`, `model/chart/price_format.rs`);
  decimal and thousands separators (`icu_decimal`) are a decision to make with the first language that
  differs.
- **First day of the week.** The date pickers start on Monday. CLDR knows each region's first day
  (`icu_calendar`'s week information); the pickers' grids need to take it.
- **The macOS menu bar and notifications** are converted (`native_bridge.rs` retitles the menus when
  `i18n::revision()` changes; the alert agent reads the language setting through
  `host::language::Follower`). What is left in them:
  - Items AppKit adds by itself (the Window menu's list of windows, Help's search field, Services'
    entries) are in the system's language, not the app's, until the bundle declares its languages
    (see "Packaging that names languages").
  - The language on a machine whose system language changes while the app or the agent runs is read
    again only at the next start; the alert agent follows a choice made in Settings within a couple of seconds.
  - A delivery failure is stored as English text in the alert history (`delivery.error`): it is
    written once, by whichever process tried, and not translated.
- **The language of the AI assistant** and of the docs site (its own repository).
- **A bare-literal lint** like the layering test's debt lists, once most of the interface is converted:
  it would fail a literal in a widget call outside a list of files that still have them.
