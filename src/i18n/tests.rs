//! Keeps the messages in the code, the `.pot` template and the catalogs in step, and checks the
//! macros end to end.

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use polib::catalog::Catalog as PoCatalog;
use polib::message::Message;
use polib::metadata::CatalogMetadata;

use super::catalog::{Catalog, Message as Translation};
use super::extract::{self, Kind, Use};
use super::template::placeholders;
use super::{Locale, Preference, tr, tr_c, tr_n, with_locale};

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
fn the_extractor_reads_every_form_the_macros_accept() {
    fn generic<A, B>() -> usize {
        std::mem::size_of::<A>() + std::mem::size_of::<B>()
    }
    let n = 2usize;
    let values = [1, 2, 3];
    let calls: [(&str, Kind, &str, &[&str]); 8] = [
        (accepted!(tr!("Save")), Kind::Plain, "Save", &[]),
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
    let mut catalog = PoCatalog::new(CatalogMetadata::new());
    // No version: a release bump would make the template stale without a message changing.
    catalog.metadata.project_id_version = "DegenView".to_owned();
    catalog.metadata.content_type = "text/plain; charset=UTF-8".to_owned();
    // English's forms: a translator's tool replaces them with the language's own.
    catalog.metadata.plural_rules.nplurals = 2;
    catalog.metadata.plural_rules.expr = "(n != 1)".to_owned();
    for ((context, msgid), (used, places)) in unique_messages(source) {
        let mut builder = match &used.kind {
            Kind::Counted(_) => Message::build_plural(),
            _ => Message::build_singular(),
        };
        builder.with_msgid(msgid).with_source(places.join(" "));
        if !context.is_empty() {
            builder.with_msgctxt(context);
        }
        if matches!(used.kind, Kind::Counted(_)) {
            builder
                .with_msgid_plural(used.key.clone())
                .with_msgstr_plural(vec![String::new(), String::new()]);
        }
        catalog.append_or_update(builder.done());
    }
    let mut out = BufWriter::new(Vec::new());
    polib::po_file::write(&catalog, &mut out).expect("a template writes to memory");
    String::from_utf8(out.into_inner().expect("flushes")).expect("the template is UTF-8")
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

fn read_template(text: &str) -> Result<polib::catalog::Catalog, String> {
    polib::po_file::parse_from_reader(std::io::Cursor::new(text.as_bytes()))
        .map_err(|error| error.to_string())
}

fn keys_in_template(template: &polib::catalog::Catalog) -> BTreeSet<Key> {
    template
        .messages()
        .map(|message| {
            (
                message.msgctxt().unwrap_or("").to_owned(),
                message.msgid().to_owned(),
                message.msgid_plural().ok().map(str::to_owned),
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
    assert_eq!(template.metadata.plural_rules.nplurals, 2);
    for message in template.messages() {
        if message.is_plural() {
            let forms = message.msgstr_plural().expect("counted");
            assert!(forms.len() == 2 && forms.iter().all(String::is_empty));
        } else {
            assert_eq!(message.msgstr().expect("plain"), "", "{}", message.msgid());
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

// MARK: Plural formulas against CLDR

/// The formula each of these languages' catalogs carries (GNU gettext's own table), against the
/// categories CLDR gives the same language: two counts have the same gettext form exactly when they
/// have the same CLDR category, for every count a screen shows.
#[test]
fn the_plural_formulas_translators_write_agree_with_cldr() {
    use super::plural_forms::PluralForms;
    use icu_plurals::PluralRules;

    let formulas: [(&str, usize, &str); 9] = [
        ("de", 2, "n != 1"),
        ("es", 2, "n != 1"),
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
