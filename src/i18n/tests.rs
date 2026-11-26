//! Keeps the messages in the code, the `.pot` template and the catalogs in step, and checks the
//! macros end to end.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ferrocat_po::{Header, MsgStr, PoFile, PoItem, SerializeOptions, parse_po, stringify_po};

use super::catalog::{Catalog, Message as Translation};
use super::extract::{self, Kind, Use};
use super::plural_forms::PluralForms;
use super::template::placeholders;
use super::{Locale, Preference, tr, tr_c, tr_n, tr_noop, with_locale};

// MARK: The macros

#[test]
fn english_is_the_text_as_written() {
    assert_eq!(tr!("Save"), "Save");
    assert_eq!(
        tr!("{symbol} crossed {price}", symbol = "BTC", price = 70_000),
        "BTC crossed 70000"
    );
    assert_eq!(tr_c!("month abbreviation", "Jan"), "Jan");
    assert_eq!(tr_c!("month", "{n} left", n = 3), "3 left");
}

#[test]
fn a_counted_message_takes_one_or_other_and_passes_its_count() {
    let alerts = |n: usize| tr_n!(n, "{count} alert", "{count} alerts");
    assert_eq!(alerts(0), "0 alerts");
    assert_eq!(alerts(1), "1 alert");
    assert_eq!(alerts(2), "2 alerts");
    assert_eq!(
        tr_n!(
            1,
            "{count} of {total} alert",
            "{count} of {total} alerts",
            total = 9
        ),
        "1 of 9 alert"
    );
    // Either form may leave the count out.
    assert_eq!(tr_n!(1, "One alert", "{count} alerts"), "One alert");
}

#[test]
fn the_pseudo_locale_changes_every_kind_of_message_and_keeps_its_values() {
    with_locale(Locale::PSEUDO, || {
        assert_eq!(tr!("Save"), "[Šåvé ~~]");
        assert_eq!(tr_c!("month abbreviation", "Jan"), "[Ĵåñ ~~]");
        assert_eq!(tr!("Hi {name}", name = "Ana"), "[Ĥî Ana ~]");
        let alerts = |n: usize| tr_n!(n, "{count} alert", "{count} alerts");
        assert_eq!(alerts(1), "[1 åļéŕţ ~~]");
        assert_eq!(alerts(3), "[3 åļéŕţš ~~~]");
    });
    assert_eq!(tr!("Save"), "Save");
}

#[test]
fn the_text_of_a_plain_message_is_not_copied() {
    assert!(std::ptr::eq(tr!("Save"), "Save"));
}

/// Compiles the call with the real macros and gives back its tokens as text, so the extractor reads
/// exactly what the compiler accepted.
macro_rules! accepted {
    ($call:expr) => {{
        let _ = $call;
        stringify!($call)
    }};
}

/// The extractor reads each call with the grammar of the macros (`extract::read_call`). The calls
/// here are compiled by the macros and read by the extractor from the same tokens, so the two cannot
/// drift apart: a form one accepts and the other refuses fails here.
#[test]
fn text_marked_for_a_static_is_translated_where_it_is_read() {
    static TABLE: [super::Marked; 2] = [tr_noop!("Save"), tr_noop!("Cancel")];
    assert_eq!(TABLE[0].text(), "Save");
    assert_eq!(TABLE[1].msgid(), "Cancel");
    with_locale(Locale::PSEUDO, || {
        assert_eq!(TABLE[0].text(), "[Šåvé ~~]");
        // The key stays English: an id built from it does not change with the language.
        assert_eq!(TABLE[0].msgid(), "Save");
    });
}

#[test]
fn the_extractor_reads_every_form_the_macros_accept() {
    fn generic<A, B>() -> usize {
        std::mem::size_of::<A>() + std::mem::size_of::<B>()
    }
    let n = 2usize;
    let values = [1, 2, 3];
    let calls: [(&str, Kind, &str, &[&str]); 9] = [
        (accepted!(tr!("Save")), Kind::Plain, "Save", &[]),
        (accepted!(tr_noop!("Save")), Kind::Plain, "Save", &[]),
        (
            accepted!(tr!("{n}", n = generic::<u8, u16>())),
            Kind::Plain,
            "{n}",
            &["n"],
        ),
        (
            accepted!(tr!(
                "{n}",
                n = values.iter().min_by(|a, b| b.cmp(a)).unwrap_or(&0)
            )),
            Kind::Plain,
            "{n}",
            &["n"],
        ),
        (
            accepted!(tr!("{a} {b}", a = 1, b = 2,)),
            Kind::Plain,
            "{a} {b}",
            &["a", "b"],
        ),
        (
            accepted!(tr!("{x}", x = tr!("Now"))),
            Kind::Plain,
            "{x}",
            &["x"],
        ),
        (
            accepted!(tr_c!("month", "May")),
            Kind::Context("month".into()),
            "May",
            &[],
        ),
        (
            accepted!(tr_c!("month", "{d} left", d = n)),
            Kind::Context("month".into()),
            "{d} left",
            &["d"],
        ),
        (
            accepted!(tr_n!(n.min(3), "One of {t}", "{count} of {t}", t = 9,)),
            Kind::Counted("One of {t}".into()),
            "{count} of {t}",
            &["count", "t"],
        ),
    ];
    for (call, kind, key, args) in calls {
        let found = extract::uses(&format!("fn f() {{ {call} }}"))
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        let first = &found[0];
        let names: Vec<&str> = first.args.iter().map(String::as_str).collect();
        assert_eq!(
            (&first.kind, first.key.as_str(), names.as_slice()),
            (&kind, key, args),
            "{call}"
        );
    }
}

// MARK: Source

fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    walk(&src, &mut found);
    // The runtime's own documentation and tests write messages that are only examples.
    let i18n = src.join("i18n");
    found.retain(|path| !path.starts_with(&i18n));
    found.sort();
    found
}

/// Where a message is written, as the template names it: the file's path under the crate with `/`
/// between its parts on every OS. (`Path::display` writes the separators of the OS it runs on;
/// `components` reads them, so the same file has the same reference on all of them.)
fn reference(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every message the app writes, with the file it is in.
fn messages_in_source() -> Vec<(String, Use)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut all = Vec::new();
    for path in source_files() {
        let text = std::fs::read_to_string(&path).expect("source reads");
        let name = reference(root, &path);
        match extract::uses(&text) {
            Ok(uses) => all.extend(uses.into_iter().map(|used| (name.clone(), used))),
            Err(error) => panic!("{name}: {error}"),
        }
    }
    all
}

#[test]
fn every_message_in_the_source_can_be_read() {
    let messages = messages_in_source();
    // A scan that found nothing would pass every check below.
    assert!(
        messages.iter().any(|(_, used)| used.key == "Theme"),
        "the scan did not find the messages in the source"
    );
}

/// What is wrong with how messages are written together, beyond what each one's compile-time check
/// says: one text must not mean two things.
#[test]
fn a_text_is_one_message_with_one_set_of_forms() {
    let mut forms: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for (_, used) in messages_in_source() {
        let shape = match &used.kind {
            Kind::Counted(_) => format!("counted: {}", used.key),
            _ => "plain".to_owned(),
        };
        forms
            .entry((used.context().to_owned(), used.msgid().to_owned()))
            .or_default()
            .insert(shape);
    }
    for ((context, msgid), shapes) in forms {
        assert_eq!(
            shapes.len(),
            1,
            "{context:?} {msgid:?} is written as {shapes:?}: a text that is counted in one place and \
             plain in another, or has two plurals, cannot share a translation; give one a context"
        );
    }
}

// MARK: The template

/// What a catalog needs of each message: one entry per context and text, in a fixed order, with
/// the files that write it. Files and not lines (`msgcat --add-location=file`): a line number
/// would make the template stale, and conflict between pull requests, whenever any line above a
/// message changes.
fn unique_messages(source: &[(String, Use)]) -> BTreeMap<(String, String), (&Use, Vec<String>)> {
    let mut unique: BTreeMap<(String, String), (&Use, Vec<String>)> = BTreeMap::new();
    for (file, used) in source {
        let entry = unique
            .entry((used.context().to_owned(), used.msgid().to_owned()))
            .or_insert_with(|| (used, Vec::new()));
        if !entry.1.contains(file) {
            entry.1.push(file.clone());
        }
    }
    unique
}

/// The `.pot` template: every message of the source, untranslated, with where it is written.
fn template(source: &[(String, Use)]) -> String {
    let header = |key: &str, value: &str| Header {
        key: key.to_owned(),
        value: value.to_owned(),
    };
    let mut file = PoFile {
        headers: vec![
            // No version: a release bump would make the template stale without a message changing.
            header("Project-Id-Version", "DegenView"),
            header("Content-Type", "text/plain; charset=UTF-8"),
            // English's forms: a translator's tool replaces them with the language's own.
            header("Plural-Forms", "nplurals=2; plural=(n != 1);"),
        ],
        ..PoFile::default()
    };
    for ((context, msgid), (used, places)) in unique_messages(source) {
        let counted = matches!(used.kind, Kind::Counted(_));
        let mut item = PoItem::new(2);
        item.msgid = msgid;
        item.msgctxt = (!context.is_empty()).then_some(context);
        item.references = places.into();
        if counted {
            item.msgid_plural = Some(used.key.clone());
            item.msgstr = MsgStr::Plural(vec![String::new(), String::new()]);
        } else {
            item.msgstr = MsgStr::Singular(String::new());
        }
        file.items.push(item);
    }
    stringify_po(&file, &SerializeOptions::default())
}

/// A message as a translator sees it: its context, its text and, when it is counted, its plural.
type Key = (String, String, Option<String>);

fn keys_in_source(source: &[(String, Use)]) -> BTreeSet<Key> {
    source
        .iter()
        .map(|(_, used)| {
            let plural = matches!(used.kind, Kind::Counted(_)).then(|| used.key.clone());
            (used.context().to_owned(), used.msgid().to_owned(), plural)
        })
        .collect()
}

fn read_template(text: &str) -> Result<PoFile, String> {
    parse_po(text).map_err(|error| error.to_string())
}

fn keys_in_template(template: &PoFile) -> BTreeSet<Key> {
    template
        .items
        .iter()
        .map(|item| {
            (
                item.msgctxt.clone().unwrap_or_default(),
                item.msgid.clone(),
                item.msgid_plural.clone(),
            )
        })
        .collect()
}

fn template_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/locales/messages.pot")
}

/// `messages.pot` is what translators start from, so it must list the messages the source writes.
/// That is the whole contract: the order, the header and the `#:` references are informational and
/// `msgmerge` (or Weblate) refreshes them, so none of them can fail this test. The test names the
/// messages that differ. `UPDATE_POT=1 cargo test --lib i18n::tests::the_template_lists` rewrites
/// the file.
#[test]
fn the_template_lists_the_messages_in_the_source() {
    let source = messages_in_source();
    let path = template_path();
    if std::env::var_os("UPDATE_POT").is_some() {
        std::fs::write(&path, template(&source)).expect("the template is written");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    let template = read_template(&committed).expect("assets/locales/messages.pot is a .po file");
    let wanted = keys_in_source(&source);
    let found = keys_in_template(&template);
    let missing: Vec<_> = wanted.difference(&found).collect();
    let extra: Vec<_> = found.difference(&wanted).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "assets/locales/messages.pot lacks {missing:#?} and lists {extra:#?}: run \
         `UPDATE_POT=1 cargo test --lib i18n::tests::the_template_lists` and commit it"
    );
}

#[test]
fn the_template_is_untranslated_and_gives_every_counted_message_two_forms() {
    let committed = std::fs::read_to_string(template_path()).expect("the template exists");
    let template = read_template(&committed).expect("a .po file");
    let header = template
        .headers
        .iter()
        .find(|header| header.key == "Plural-Forms")
        .expect("the template says its plural forms");
    assert_eq!(
        PluralForms::from_header(&header.value)
            .expect("valid")
            .count(),
        2
    );
    for item in &template.items {
        match &item.msgstr {
            MsgStr::Plural(forms) => {
                assert!(forms.len() == 2 && forms.iter().all(String::is_empty));
            }
            MsgStr::Singular(text) => assert_eq!(text, "", "{}", item.msgid),
            MsgStr::None => {}
        }
    }
}

#[test]
fn a_reference_is_the_path_under_the_crate_with_slashes() {
    let root = Path::new("repo");
    assert_eq!(
        reference(root, &root.join("src").join("ui").join("x.rs")),
        "src/ui/x.rs"
    );
    assert_eq!(reference(root, Path::new("lib.rs")), "lib.rs");
}

/// The separators of Windows are read there, and a path never carries the other OS's.
#[cfg(windows)]
#[test]
fn a_windows_path_has_the_same_reference_as_on_any_other_os() {
    let root = Path::new(r"C:\repo");
    assert_eq!(
        reference(root, Path::new(r"C:\repo\src\ui\x.rs")),
        "src/ui/x.rs"
    );
}

// MARK: The catalogs

/// What is wrong with a catalog beside the messages in the source.
fn problems_in_catalog(catalog: &Catalog, source: &[(String, Use)]) -> Vec<String> {
    let unique = unique_messages(source);
    let mut problems = Vec::new();
    for (context, msgid, translation) in catalog.entries() {
        let Some((used, _)) = unique.get(&(context.to_owned(), msgid.to_owned())) else {
            problems.push(format!(
                "{context:?} {msgid:?} is not a message in the source"
            ));
            continue;
        };
        let counted = matches!(used.kind, Kind::Counted(_));
        let texts: Vec<&str> = match translation {
            Translation::Text(text) => vec![text],
            Translation::Plural(forms) => forms.clone(),
        };
        if matches!(translation, Translation::Plural(_)) != counted {
            problems.push(format!(
                "{msgid:?} is counted in one place and not in the other"
            ));
        }
        // A translation uses the values the source passes, and a plain one all of them.
        let allowed: BTreeSet<&str> = used.args.iter().map(String::as_str).collect();
        for text in &texts {
            for name in placeholders(text) {
                if !allowed.contains(name) {
                    problems.push(format!(
                        "{{{name}}} in the translation of {msgid:?} is not one of its values"
                    ));
                }
            }
        }
        if !counted {
            let wanted: BTreeSet<&str> = placeholders(msgid).into_iter().collect();
            let given: BTreeSet<&str> = placeholders(texts[0]).into_iter().collect();
            if wanted != given {
                problems.push(format!(
                    "the translation of {msgid:?} does not use the same values"
                ));
            }
        }
    }
    problems
}

#[test]
fn every_catalog_matches_the_messages_in_the_source() {
    let source = messages_in_source();
    for locale in Locale::all() {
        let Some(po) = locale.catalog_source() else {
            continue;
        };
        let catalog =
            Catalog::parse(po).unwrap_or_else(|error| panic!("{}: {error}", locale.tag()));
        let problems = problems_in_catalog(&catalog, &source);
        assert!(
            problems.is_empty(),
            "{}:\n{}",
            locale.tag(),
            problems.join("\n")
        );
    }
}

fn used(kind: Kind, key: &str, args: &[&str]) -> (String, Use) {
    (
        "src/x.rs".to_owned(),
        Use {
            line: 1,
            kind,
            key: key.to_owned(),
            args: args.iter().map(|name| (*name).to_owned()).collect(),
        },
    )
}

fn russian(body: &str) -> Catalog {
    Catalog::parse(&format!(
        "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\
         \"Plural-Forms: nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && \
         (n%100<10 || n%100>=20) ? 1 : 2);\\n\"\n\n{body}"
    ))
    .expect("parses")
}

#[test]
fn a_catalog_that_drifted_from_the_source_is_reported() {
    let source = [
        used(Kind::Plain, "Save", &[]),
        used(Kind::Plain, "Hi {name}", &["name"]),
        used(
            Kind::Counted("{count} alert".into()),
            "{count} alerts",
            &["count"],
        ),
        used(Kind::Context("month".into()), "Jan", &[]),
    ];
    let good = russian(
        r#"msgid "Save"
msgstr "Сохранить"

msgid "Hi {name}"
msgstr "Привет, {name}"

msgctxt "month"
msgid "Jan"
msgstr "янв"

msgid "{count} alert"
msgid_plural "{count} alerts"
msgstr[0] "{count} оповещение"
msgstr[1] "{count} оповещения"
msgstr[2] "{count} оповещений"
"#,
    );
    assert_eq!(problems_in_catalog(&good, &source), Vec::<String>::new());

    let stale = russian("msgid \"Gone\"\nmsgstr \"Ушло\"\n");
    assert_eq!(problems_in_catalog(&stale, &source).len(), 1);

    let wrong_value = russian("msgid \"Hi {name}\"\nmsgstr \"Привет, {nombre}\"\n");
    assert_eq!(problems_in_catalog(&wrong_value, &source).len(), 2);

    let wrong_context = russian("msgid \"Jan\"\nmsgstr \"янв\"\n");
    assert_eq!(problems_in_catalog(&wrong_context, &source).len(), 1);

    let not_counted = russian("msgid \"{count} alert\"\nmsgstr \"{count} оповещение\"\n");
    assert_eq!(problems_in_catalog(&not_counted, &source).len(), 1);
}

#[test]
fn the_template_lists_each_message_once_with_its_places_and_forms() {
    let source = [
        used(Kind::Plain, "Save", &[]),
        used(Kind::Plain, "Save", &[]),
        used(
            Kind::Counted("{count} alert".into()),
            "{count} alerts",
            &["count"],
        ),
        used(Kind::Context("month".into()), "May", &[]),
    ];
    let text = template(&source);
    assert_eq!(text.matches("msgid \"Save\"").count(), 1);
    assert!(text.contains("msgid \"{count} alert\"\nmsgid_plural \"{count} alerts\""));
    assert!(text.contains("msgctxt \"month\"\nmsgid \"May\""));
    assert!(text.contains("#: src/x.rs\n"));
    assert!(!text.contains("src/x.rs:1"));
}

#[test]
fn a_choice_that_names_a_language_the_build_lacks_is_not_a_locale() {
    assert!(Locale::from_tag("tlh").is_none());
    assert_eq!(Preference::default(), Preference::System);
}

// MARK: GNU gettext compatibility

const LAYOUT_HEADER: &str = "msgid \"\"\nmsgstr \"\"\n\
    \"Content-Type: text/plain; charset=UTF-8\\n\"\n\
    \"Language: es\\n\"\n\
    \"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n";

/// One catalog of three messages (`Save`, `May` in the context `month`, and a counted one), laid
/// out the ways GNU gettext accepts: its tools write the first, people and other editors write the
/// rest. A reader that ends an entry anywhere but at the next `msgid`/`msgctxt` reads the others
/// wrongly or not at all.
fn gnu_layouts() -> Vec<(&'static str, String)> {
    let body = |separator: &str, comment: &str| {
        format!(
            "{comment}msgid \"Save\"\nmsgstr \"Guardar\"\n{separator}\
             {comment}msgctxt \"month\"\nmsgid \"May\"\nmsgstr \"mayo\"\n{separator}\
             {comment}msgid \"{{count}} alert\"\nmsgid_plural \"{{count}} alerts\"\n\
             msgstr[0] \"{{count}} alerta\"\nmsgstr[1] \"{{count}} alertas\"\n"
        )
    };
    let canonical = format!("{LAYOUT_HEADER}\n{}", body("\n", "#: src/a.rs\n"));
    let no_blank_lines = format!("{LAYOUT_HEADER}{}", body("", "#: src/a.rs\n"));
    let crlf = canonical.replace('\n', "\r\n");
    // Translator and extracted comments, flags, a previous msgid, and an obsolete entry.
    let annotated = format!(
        "{LAYOUT_HEADER}\n#. A button\n# translator note\n#, c-format\n#| msgid \"Store\"\n{}\n\
         #~ msgid \"Gone\"\n#~ msgstr \"Ido\"\n",
        body("\n", "")
    );
    // Strings broken over lines, as `xgettext` and `msgmerge` wrap long ones.
    let wrapped = canonical
        .replace("msgstr \"Guardar\"", "msgstr \"\"\n\"Guar\"\n\"dar\"")
        .replace("msgid \"Save\"", "msgid \"\"\n\"Sa\"\n\"ve\"");
    vec![
        ("canonical", canonical),
        ("no blank lines", no_blank_lines),
        ("CRLF line ends", crlf),
        ("comments, flags, previous and obsolete entries", annotated),
        ("strings over several lines", wrapped),
    ]
}

#[test]
fn every_layout_gnu_gettext_accepts_reads_as_the_same_catalog() {
    let entries = |po: &str| -> Vec<(String, String, String)> {
        let catalog = Catalog::parse(po).unwrap_or_else(|error| panic!("{error}\n{po}"));
        let mut entries: Vec<_> = catalog
            .entries()
            .map(|(context, key, message)| {
                (context.to_owned(), key.to_owned(), format!("{message:?}"))
            })
            .collect();
        entries.sort();
        entries
    };
    let layouts = gnu_layouts();
    let expected = entries(&layouts[0].1);
    assert_eq!(expected.len(), 3, "{expected:?}");
    for (name, po) in &layouts {
        assert_eq!(entries(po), expected, "{name}");
    }
}

/// GNU `msgfmt` is the judge of what is valid gettext: it must accept the layouts above (so they are
/// not only our reading of the format) and every catalog and template the app ships. Needs gettext's
/// tools on the `PATH` (CI's Linux image has them); without them the test says so and passes.
#[test]
fn the_catalogs_and_layouts_are_valid_gettext() {
    use std::process::Command;
    let msgfmt = |file: &Path| {
        Command::new("msgfmt")
            .args(["--check-format", "--output-file=-"])
            .arg(file)
            .output()
    };
    let directory = std::env::temp_dir().join(format!("degenview-gettext-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a scratch folder");
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for (index, (name, po)) in gnu_layouts().into_iter().enumerate() {
        let file = directory.join(format!("layout-{index}.po"));
        std::fs::write(&file, po).expect("written");
        files.push((name.to_owned(), file));
    }
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/locales");
    for entry in std::fs::read_dir(shipped)
        .expect("the catalogs folder")
        .flatten()
    {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "po" || extension == "pot")
        {
            files.push((path.display().to_string(), path));
        }
    }
    let mut checked = 0;
    for (name, file) in &files {
        match msgfmt(file) {
            Ok(output) => {
                assert!(
                    output.status.success(),
                    "msgfmt refuses {name}:\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                checked += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("msgfmt is not installed: gettext compatibility not checked");
                break;
            }
            Err(error) => panic!("msgfmt could not run: {error}"),
        }
    }
    let _ = std::fs::remove_dir_all(&directory);
    assert!(checked == 0 || checked == files.len());
}

// MARK: Plural formulas against CLDR

/// The formula each of these languages' catalogs carries (GNU gettext's own table), against the
/// categories CLDR gives the same language: two counts have the same gettext form exactly when they
/// have the same CLDR category, for every count a screen shows.
#[test]
fn the_plural_formulas_translators_write_agree_with_cldr() {
    use super::plural_forms::PluralForms;
    use icu_plurals::PluralRules;

    let formulas: [(&str, usize, &str); 10] = [
        ("de", 2, "n != 1"),
        ("es", 2, "n != 1"),
        ("zh", 1, "0"),
        ("fr", 2, "n > 1"),
        ("pt-BR", 2, "n > 1"),
        ("ja", 1, "0"),
        ("tr", 2, "n != 1"),
        (
            "ru",
            3,
            "n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2",
        ),
        (
            "pl",
            3,
            "n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2",
        ),
        (
            "ar",
            6,
            "n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5",
        ),
    ];
    for (tag, count, formula) in formulas {
        let forms = PluralForms::parse(count, formula).expect("valid formula");
        let rules = PluralRules::try_new_cardinal(
            tag.parse::<icu_locale_core::Locale>().expect("tag").into(),
        )
        .expect("CLDR has the language");
        let mut index_of_category = BTreeMap::new();
        let mut category_of_index = BTreeMap::new();
        for n in 0..=10_000u64 {
            let category = format!("{:?}", rules.category_for(n));
            let index = forms.index(n);
            assert_eq!(
                *index_of_category.entry(category.clone()).or_insert(index),
                index,
                "{tag}: {n} is {category} in CLDR but takes another form than its siblings"
            );
            assert_eq!(
                category_of_index.entry(index).or_insert(category.clone()),
                &category,
                "{tag}: {n} shares form {index} with a different CLDR category"
            );
        }
        // Polish and Arabic use categories a 10,000 count does not reach (`many` of 1.5): the
        // forms the formula has are the categories the counts reach, plus none.
        assert!(
            category_of_index.len() <= count,
            "{tag}: the counts reach more forms than nplurals says"
        );
    }
}

// MARK: Names in sentences

#[test]
fn a_name_inside_a_sentence_keeps_its_capital_only_where_nouns_have_one() {
    use super::{Locale, in_sentence, with_locale};
    with_locale(Locale::ENGLISH, || {
        assert_eq!(in_sentence("Trend line"), "trend line");
    });
    with_locale(Locale::from_tag("de").expect("German is offered"), || {
        assert_eq!(in_sentence("Trendlinie"), "Trendlinie");
    });
}

// MARK: Lists

#[test]
fn a_list_is_joined_the_way_the_language_joins_it() {
    use super::{Locale, join_list, with_locale};
    let items = ["Mon", "Tue", "Wed"];
    with_locale(Locale::ENGLISH, || {
        assert_eq!(join_list(&items), "Mon, Tue, and Wed");
        assert_eq!(join_list(&items[..2]), "Mon and Tue");
        assert_eq!(join_list(&items[..1]), "Mon");
        assert_eq!(join_list::<&str>(&[]), "");
    });
    with_locale(Locale::from_tag("de").expect("German is offered"), || {
        assert_eq!(join_list(&items), "Mon, Tue und Wed");
    });
    with_locale(Locale::from_tag("es").expect("Spanish is offered"), || {
        assert_eq!(join_list(&items), "Mon, Tue y Wed");
    });
    with_locale(Locale::from_tag("zh").expect("Chinese is offered"), || {
        assert_eq!(join_list(&items), "Mon、Tue和Wed");
        assert_eq!(join_list(&items[..2]), "Mon和Tue");
    });
    // The pseudo-locale writes lists as English, like its dates.
    with_locale(Locale::PSEUDO, || {
        assert_eq!(join_list(&items), "Mon, Tue, and Wed")
    });
}

#[test]
fn sentences_are_joined_with_the_space_their_language_puts_between_them() {
    use super::join_sentences;
    assert_eq!(
        join_sentences(&["One.", "Two.", "Three"]),
        "One. Two. Three"
    );
    assert_eq!(
        join_sentences(&["一。", "二！", "三？", "四"]),
        "一。二！三？四"
    );
    assert_eq!(join_sentences(&["", "Only."]), "Only.");
    assert_eq!(join_sentences::<&str>(&[]), "");
}
