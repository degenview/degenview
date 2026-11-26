# Adding or updating a translation

For an agent asked to add an interface language, or to bring one up to date. How the i18n runtime
works (the macros, plurals, dates, fallback) is in [Localization](../localization.md); this is the
procedure and the quality bar. Spanish (`assets/locales/es.po`, PR #741) is the worked example.

## The standard: translate the place, not the words

A translation is judged where it appears. "Open" is *Abrir* on a button and *Apertura* over a price
column; word for word is wrong in one of them.

- **Read the call site of every message.** Each `#:` line in `messages.pot` names a file; open it and
  find the English text. Decide what the string is (button, menu item, column header, badge, chip,
  tab, section title, tooltip, empty state, error sentence, status line) and write it as that. Length
  follows the role: a column header, badge or toolbar label is at most about 30% longer than the
  English (shorter, natural words win over literal ones); a tooltip or help sentence may take its
  natural length.
- **`msgctxt` says which sense applies.** Honour it. Two places that need different words for the same
  English get a context in the source (`tr_c!("price", "Open")`), never a shared msgid. A catalog
  can only translate a message once: if you catch two meanings under one msgid, split it in the
  source first, regenerate the template, then translate both. Words that already needed this:
  Open/Close (price, order status, button), New/Fixed (release notes, commission), Stop (button,
  order type).
- **One term, one translation, everywhere.** Write a glossary before translating (below) and use it
  for every entry; grep the finished catalog for each English term and fix drift (Test as a button
  and as a noun, Account, Position, Holdings).
- **One mood per string, chosen by role.** Where the language has a command form for the reader (Spain
  Spanish: buttons, menu items and tooltips that name an action take the infinitive, "Guardar"; a heading
  or hint that talks to the person takes the informal imperative, "Inicia un gráfico nuevo", "Añade una
  transacción"), pick by what the text is, not by the English verb. Never mix the two inside one string
  ("Mostrar detalles · arrastra…"). After translating, list every message that starts with a command
  verb and read the headings, hints and empty-state texts among them.
- **Mirror the English's form.** Sentence case stays sentence case; a trailing `…` or `:` stays; a
  label stays a label and a sentence a sentence. Unicode quotes, `·`, `→`, `×`, `⌘ ⌥ ⇧` stay.
- **Placeholders are exact.** Keep every `{name}` byte for byte; reorder them if the language needs
  it, never rename, translate or drop one. Counted messages (`msgid_plural`) get every plural form
  the language has (`Plural-Forms` header), each with `{count}` where the English has it.
- **Do not translate what is not interface text** (the rules are in
  [Localization](../localization.md#writing-text)): tickers, exchange/provider/broker names, product
  names (DegenView, Pine, CloudKit…), Pine identifiers and source, JSON keys, file formats, units,
  timeframe codes (`1m`, `4h`, `1D`), key caps, names a person gave. When an entry is such a thing,
  copy the msgid into the msgstr (never leave it empty: empty means "English" and hides a gap you
  cannot tell from a miss).
- **Look for code that builds language.** English ordinals ("3rd"), `if n == 1 { "item" }`, a
  sentence joined from `tr!` pieces, a platform word put into a sentence. Another language cannot
  translate these: change the source to one whole message (`tr_n!`, one `tr!` per platform, the value
  as a placeholder), then translate it. Fix it at the code, not in the catalog.
- **English that never reached the catalog has three usual homes**, each with its own fix: a `static`
  table of text (`tr_noop!` and `.text()` where it is drawn), a sentence stored when something happened
  (store what happened and write the sentence when it is shown, as the paper journal's `PaperNote` does;
  the stored English stays for exports and MCP), and an error type's `#[error(...)]` (a helper function
  that calls `tr!`, because `thiserror` takes a call and the extractor wants plain expressions).
- **Terms of the trade** that people in that language use in English stay English (Long, Short, Stop
  loss, Take profit, Bid/Ask, Webhook, Footprint, VWAP…). Decide the list per language, in the glossary.
- **Platform wording follows the platform's own language**: for the macOS menu bar use the system's
  names (Archivo, Edición, Visualización, Ventana, Ayuda in Spain), not your own.
- **Regional variant.** Pick the variant the tag asks for and say it in the glossary (Spain: informal
  *tú*, infinitive on buttons, *ordenador*, *Ajustes*). Tag the file with the language only (`es`), not
  the region (`es-ES`): `Locale::negotiate` falls back along CLDR's chain, so `es-MX` and `es-419` land
  on `es`. A region-specific file is for a language that really has two written forms
  (`zh`/`zh-Hant`, `pt-BR`/`pt-PT`; CLDR's chain makes Simplified the plain `zh`).

## Files to change

| File | Change |
| --- | --- |
| `assets/locales/<tag>.po` | New. Start from `msginit --no-translator -l <tag> -i assets/locales/messages.pot -o assets/locales/<tag>.po` so the header (`Language`, `Plural-Forms`) is right. |
| `src/i18n/locale.rs` | One row in `LANGUAGES` (`tag`, `native_name` in the language itself, `Kind::Natural`, `catalog: Some(include_str!(…))`); grow the array length. Keep the pseudo-locale's index right (`Locale::PSEUDO`) and the test that indexes past the table. |
| `src/i18n/tests.rs` | The language's plural formula in `the_plural_formulas_translators_write_agree_with_cldr` (checked against CLDR). |
| `assets/locales/messages.pot` | Only when you changed source text or added a context: `UPDATE_POT=1 cargo test --lib i18n::tests::the_template_lists`. |
| Source with a shared msgid | `tr_c!` split, or the code that built language (see above). |
| `src/ui/theme/mod.rs` (`base_fonts`) | Only for a script egui's fonts lack (CJK, Thai, Arabic, Hebrew, Indic). Latin, Cyrillic and Greek need nothing. |
| `tools/bundle.sh` | `CFBundleLocalizations` in both `Info.plist` files. |
| `packaging/windows/degenview.iss` | `[Languages]` entry and translated `[CustomMessages]` / shortcut names. |
| `packaging/flatpak/es.nico.degenview.metainfo.xml` and `.desktop` | `xml:lang` summary and description, `Comment[<tag>]`. |
| `docs/localization.md` | The State paragraph, and the language's subsection (wording decisions, what is unverified). |
| `changes/<slug>.md` | `new: DegenView is available in <language>. …` ([changelog rules](releases-updates.md#release-notes-and-the-changelog)). |

No code is written per language beyond the row: plural rules and date patterns come from the catalog's
header and CLDR. Right-to-left languages are not supported yet (see "Not done yet" in the
localization doc); do not add one without that work.

## Method for a full catalog (about 5,000 messages)

One agent cannot hold the catalog and keep its wording consistent; split the work and share the
vocabulary.

1. **Write the glossary first** (in your scratchpad, not committed): the region and register, the
   style rules above, and a table of the core vocabulary (chart, alert, watchlist, portfolio,
   holdings, order, position, fill, timeframe, drawing tools, Pine Studio panels, menu-bar items).
   The Spanish one is the model: every term the product uses repeatedly, plus the "stays English"
   list and the platform menu names.
2. **Split the template** into chunks of about 500 messages, sorted by `#:` file so one chunk is one
   screen or area (a reviewer of "Settings" sees Settings together), with `msgcat` or a script;
   keep each chunk's header.
3. **One agent per chunk**, in parallel, each given the glossary, the style rules in this guide and
   the instruction to open the call site of every entry. Ask for a `.po` fragment back with every
   entry filled, each `msgstr` under its own `msgid`. Never take a numbered list of translations and
   join it back by position: two entries trade places, both still read as good text, and no gettext
   tool notices (the German proxy hints sent Mac users to Windows Settings that way).
4. **Merge and check:**

   ```bash
   msgcat --use-first --no-wrap chunk*.po -o merged.po
   msgmerge --no-fuzzy-matching --no-wrap -q merged.po assets/locales/messages.pot -o assets/locales/<tag>.po
   msgfmt -c --check-format --statistics -o /dev/null assets/locales/<tag>.po
   ```

   Then count empty `msgstr`s (there must be none) and compare plural forms to the header.
5. **Consistency pass (yours, not an agent's).** Agents that never saw each other's chunk disagree:
   - grep for each glossary term and each English word that carries several meanings, and make the
     renderings agree where the meaning is the same;
   - read every message that several chunks produced (same msgid cannot, but near-duplicates can);
   - sample at least 100 entries against their call sites, favouring short labels;
   - look for English left in a translation, untranslated placeholders, doubled spaces, a changed
     `…`.
   Then have readers who did not translate go through **every** pair of English and translation,
   asking only whether the second says what the first says (swapped, omitted, reversed, wrong
   sense). Tests catch placeholders and platform names; nothing else catches a misplaced entry.
6. **Contexts.** Anything the pass shows as one msgid with two meanings goes back to the source
   (`tr_c!`), then to the template, then to the catalog. Never settle for a compromise word.

For an update (new messages since the catalog was made): `msgmerge -U assets/locales/<tag>.po
assets/locales/messages.pot`, translate the new and `fuzzy` entries by the same rules, remove what
`msgmerge` marked obsolete (`msgattrib --no-obsolete`). A `fuzzy` entry shows in English until it is
reviewed and the flag removed, so never leave a guess flagged and never remove the flag unreviewed.

## Verify

- `cargo test --lib i18n`: the catalog and the source agree (no stale or untranslated message, same
  placeholders, consistent plural use), plural formulas match CLDR, the file is valid to `msgfmt`
  (which needs GNU gettext installed).
- **Nothing may be left untranslated.** An entry that is missing, empty or `fuzzy` shows in English;
  `every_catalog_matches_the_messages_in_the_source` fails on it. For a language not yet in
  `LANGUAGES`, which no test reads, `msgattrib --untranslated --no-obsolete assets/locales/<tag>.po`
  must print nothing.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.
- **Look at screens in the language**, not only the catalog. `DEGENVIEW_LOCALE=<tag>` names the
  language for one run (over the saved choice and the system's). Use a release build and a scratch
  `HOME` with a copy of the data folder; never a branch build on the real store
  ([Core guidance](core.md#build-and-test)). Open Settings (every page), a chart with the toolbar and
  its menus, the symbol picker (⌘K), Portfolio, Pine Studio, the paper and live trading sheets, the
  alert drawer, context menus, and the macOS menu bar; at the narrowest window width. Look for text
  cut with an ellipsis, columns whose values clip, buttons wider than their row.
- When a window cannot be captured (no Screen Recording or Accessibility permission for the shell),
  render pages through the gallery tests instead: `#[ignore]`d tests that draw real egui frames to
  PNG with `DEGENVIEW_GALLERY_DIR=<dir>`; they run in the language set in `Screen::new`. Say in the PR
  what was and was not looked at.
- **Text that stays English in a real window is a finding.** It is either a message missing from the
  catalog or a literal that never went through `tr!` (the blank tab's title was one): fix the first in
  the catalog, the second in the source (`tr!`, then the template and every catalog).
- A label that does not fit is shortened in the translation first; widen the layout only when the
  width was tuned to English (widths are measured from the words, see Localization).
- The pseudo-locale (`DEGENVIEW_LOCALE=en-XA`, debug builds) finds text that never went through
  `tr!` and fixed widths that are too small. Its page tests assert bracketed text, so they cannot be
  pointed at a real language.

## Hand-off

State in the PR what is not verified: no native review (ask for one before a release candidate),
packaging checked for syntax only on Windows and Linux, screens not seen. Editor theme names,
numbers (one format for every language) and the first day of the week are decisions recorded in
[Localization](../localization.md#not-done-yet), not part of a translation PR.
