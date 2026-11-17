//! Keeps the messages in the code and the catalogs in step, and checks the macros end to end.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::catalog::{Entry, File};
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
}

#[test]
fn the_pseudo_locale_changes_every_kind_of_message_and_keeps_its_values() {
    with_locale(Locale::Pseudo, || {
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

// MARK: Source and catalogs

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
    // The macros' own documentation and tests write messages that are only examples; the calendar's
    // names are real messages.
    let i18n = src.join("i18n");
    found.retain(|path| !path.starts_with(&i18n) || path.ends_with("calendar.rs"));
    found.sort();
    found
}

/// Every message the app writes, with where.
fn messages_in_source() -> Vec<(String, Use)> {
    let mut all = Vec::new();
    for path in source_files() {
        let text = std::fs::read_to_string(&path).expect("source reads");
        let name = path.display().to_string();
        match extract::uses(&text) {
            Ok(uses) => all.extend(uses.into_iter().map(|used| (name.clone(), used))),
            Err(error) => panic!("{name}: {error}"),
        }
    }
    all
}

#[test]
fn every_message_in_the_source_can_be_read_and_passes_the_values_it_names() {
    let messages = messages_in_source();
    // A scan that found nothing would pass every check below.
    assert!(
        messages.iter().any(|(_, used)| used.key == "Theme"),
        "the scan did not find the messages in the source"
    );
    let mut problems = Vec::new();
    for (file, used) in messages {
        problems.extend(
            problems_with(&used)
                .into_iter()
                .map(|problem| format!("{file}:{}: {problem}", used.line)),
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// What is wrong with how one message is written.
fn problems_with(used: &Use) -> Vec<String> {
    let mut problems = Vec::new();
    if used.key.trim().is_empty() {
        problems.push("the message is empty".to_owned());
    }
    // A translator is not shown spaces around text, and a catalog cannot be sure to keep them.
    if used.key.trim() != used.key {
        problems.push(format!("{:?} starts or ends with a space", used.key));
    }
    let wanted: BTreeSet<&str> = placeholders(&used.key).into_iter().collect();
    let given: BTreeSet<&str> = used.args.iter().map(String::as_str).collect();
    for missing in wanted.difference(&given) {
        problems.push(format!("{{{missing}}} in {:?} is given no value", used.key));
    }
    let counted = matches!(used.kind, Kind::Counted(_));
    for unused in given.difference(&wanted) {
        // A counted message may leave its count out of one form ("One alert").
        if !(counted && *unused == "count") {
            problems.push(format!(
                "{unused} is passed but {:?} has no {{{unused}}}",
                used.key
            ));
        }
    }
    if let Kind::Counted(one) = &used.kind {
        for name in placeholders(one) {
            if !given.contains(name) {
                problems.push(format!("{{{name}}} in {one:?} is given no value"));
            }
        }
    }
    problems
}

#[test]
fn one_counted_message_is_not_written_with_two_singulars() {
    let mut singulars: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (_, used) in messages_in_source() {
        if let Kind::Counted(one) = used.kind {
            singulars.entry(used.key).or_default().insert(one);
        }
    }
    for (other, ones) in singulars {
        assert_eq!(ones.len(), 1, "{other:?} is the plural of {ones:?}");
    }
}

/// What is wrong with a catalog beside the messages in the source.
fn problems_in_catalog(file: &File, source: &[Use]) -> Vec<String> {
    let mut known: BTreeMap<(String, String), &Use> = BTreeMap::new();
    for used in source {
        known.insert((used.context().to_owned(), used.key.clone()), used);
    }
    let mut problems = Vec::new();
    let entries = file
        .messages
        .iter()
        .map(|(key, entry)| ("", key, entry))
        .chain(file.contexts.iter().flat_map(|(context, entries)| {
            entries
                .iter()
                .map(move |(key, entry)| (context.as_str(), key, entry))
        }));
    for (context, key, entry) in entries {
        let Some(used) = known.get(&(context.to_owned(), key.clone())) else {
            problems.push(format!(
                "{context:?} {key:?} is not a message in the source"
            ));
            continue;
        };
        let allowed: BTreeSet<&str> = used.args.iter().map(String::as_str).collect();
        let texts: Vec<&String> = match entry {
            Entry::Text(text) => vec![text],
            Entry::Plural(forms) => forms.values().collect(),
        };
        if matches!(entry, Entry::Plural(_)) != matches!(used.kind, Kind::Counted(_)) {
            problems.push(format!(
                "{key:?} is counted in one place and not in the other"
            ));
        }
        for text in texts {
            for name in placeholders(text) {
                if !allowed.contains(name) {
                    problems.push(format!(
                        "{{{name}}} in the translation of {key:?} is not one of its values"
                    ));
                }
            }
        }
        if !matches!(entry, Entry::Plural(_)) {
            let wanted: BTreeSet<&str> = placeholders(key).into_iter().collect();
            let Entry::Text(text) = entry else { continue };
            let given: BTreeSet<&str> = placeholders(text).into_iter().collect();
            if wanted != given {
                problems.push(format!(
                    "the translation of {key:?} does not use the same values"
                ));
            }
        }
    }
    problems
}

#[test]
fn every_catalog_matches_the_messages_in_the_source() {
    let source: Vec<Use> = messages_in_source()
        .into_iter()
        .map(|(_, used)| used)
        .collect();
    for locale in Locale::ALL {
        let Some(json) = super::catalog::source(locale) else {
            continue;
        };
        let file = File::parse(json).unwrap_or_else(|error| panic!("{}: {error}", locale.tag()));
        let problems = problems_in_catalog(&file, &source);
        assert!(
            problems.is_empty(),
            "{}:\n{}",
            locale.tag(),
            problems.join("\n")
        );
    }
}

fn used(kind: Kind, key: &str, args: &[&str]) -> Use {
    Use {
        line: 1,
        kind,
        key: key.to_owned(),
        args: args.iter().map(|name| (*name).to_owned()).collect(),
    }
}

#[test]
fn a_message_that_misses_or_adds_a_value_is_reported() {
    assert!(problems_with(&used(Kind::Plain, "Save", &[])).is_empty());
    assert!(problems_with(&used(Kind::Plain, "{a} {b}", &["a", "b"])).is_empty());
    assert_eq!(
        problems_with(&used(Kind::Plain, "{a} {b}", &["a"])).len(),
        1
    );
    assert_eq!(
        problems_with(&used(Kind::Plain, "{a}", &["a", "b"])).len(),
        1
    );
    assert_eq!(problems_with(&used(Kind::Plain, " Save", &[])).len(), 1);
    assert_eq!(problems_with(&used(Kind::Plain, "", &[])).len(), 1);
}

#[test]
fn a_counted_message_may_leave_out_its_count_in_one_form() {
    let counted = Kind::Counted("One alert".to_owned());
    assert!(problems_with(&used(counted, "{count} alerts", &["count"])).is_empty());
    let counted = Kind::Counted("One alert".to_owned());
    assert!(problems_with(&used(counted, "Many alerts", &["count"])).is_empty());
    let counted = Kind::Counted("{count} of {n}".to_owned());
    assert_eq!(
        problems_with(&used(counted, "{count}", &["count"])).len(),
        1
    );
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
    let good = File::parse(
        r#"{"messages":{"Save":"Guardar","Hi {name}":"Hola {name}",
            "{count} alerts":{"one":"{count} alerta","other":"{count} alertas"}},
            "contexts":{"month":{"Jan":"ene"}}}"#,
    )
    .expect("parses");
    assert_eq!(problems_in_catalog(&good, &source), Vec::<String>::new());

    let stale = File::parse(r#"{"messages":{"Gone":"Ido"}}"#).expect("parses");
    assert_eq!(problems_in_catalog(&stale, &source).len(), 1);

    let wrong_value = File::parse(r#"{"messages":{"Hi {name}":"Hola {nombre}"}}"#).expect("parses");
    assert_eq!(problems_in_catalog(&wrong_value, &source).len(), 2);

    let wrong_context = File::parse(r#"{"messages":{"Jan":"ene"}}"#).expect("parses");
    assert_eq!(problems_in_catalog(&wrong_context, &source).len(), 1);

    let not_counted =
        File::parse(r#"{"messages":{"{count} alerts":"{count} alertas"}}"#).expect("parses");
    assert_eq!(problems_in_catalog(&not_counted, &source).len(), 1);
}

#[test]
fn a_choice_that_names_a_language_the_build_lacks_is_not_a_locale() {
    assert!(Locale::from_tag("tlh").is_none());
    assert_eq!(Preference::default(), Preference::System);
}
