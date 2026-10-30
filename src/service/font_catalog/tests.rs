use super::*;
use std::fs;

struct Spec<'a> {
    family: &'a str,
    subfamily: &'a str,
    mono: bool,
    weight: u16,
    selection: u16,
    /// Scalable outlines (a `glyf` table) rather than bitmaps only (`bdat`).
    outlines: bool,
    /// Names written for the Macintosh platform (ASCII) only, as Menlo's and Monaco's are.
    mac_names: bool,
}

impl<'a> Spec<'a> {
    fn regular(family: &'a str) -> Self {
        Self {
            family,
            subfamily: "Regular",
            mono: true,
            weight: 400,
            selection: 0x40,
            outlines: true,
            mac_names: false,
        }
    }
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// A font file with just the tables the catalog reads. Table offsets are absolute, so `base` is
/// where the font starts in the file it is going into.
fn font(spec: &Spec, base: u32) -> Vec<u8> {
    let (family, subfamily, platform, encoding, language) = if spec.mac_names {
        (
            spec.family.as_bytes().to_vec(),
            spec.subfamily.as_bytes().to_vec(),
            1u16,
            0u16,
            0u16,
        )
    } else {
        (utf16(spec.family), utf16(spec.subfamily), 3, 1, 0x409)
    };
    let mut names = Vec::new();
    names.extend(0u16.to_be_bytes()); // format
    names.extend(2u16.to_be_bytes()); // count
    names.extend((6u16 + 24).to_be_bytes()); // string storage
    for (id, length, offset) in [
        (1u16, family.len(), 0usize),
        (2, subfamily.len(), family.len()),
    ] {
        for value in [
            platform,
            encoding,
            language,
            id,
            length as u16,
            offset as u16,
        ] {
            names.extend(value.to_be_bytes());
        }
    }
    names.extend(&family);
    names.extend(&subfamily);

    let mut os2 = vec![0u8; 78];
    os2[4..6].copy_from_slice(&spec.weight.to_be_bytes());
    os2[62..64].copy_from_slice(&spec.selection.to_be_bytes());
    let mut post = vec![0u8; 32];
    post[12..16].copy_from_slice(&u32::from(spec.mono).to_be_bytes());

    // The tables are only found by their tags, not parsed: this stands in for the glyphs.
    let glyphs = vec![0u8; 8];
    let glyph_tag: &[u8; 4] = if spec.outlines { b"glyf" } else { b"bdat" };
    let data_start = base + 12 + 4 * 16;
    let mut out = vec![0, 1, 0, 0];
    out.extend(4u16.to_be_bytes());
    out.extend([0u8; 6]);
    let mut offset = data_start;
    for (tag, bytes) in [
        (b"OS/2", &os2),
        (glyph_tag, &glyphs),
        (b"name", &names),
        (b"post", &post),
    ] {
        out.extend(tag);
        out.extend(0u32.to_be_bytes());
        out.extend(offset.to_be_bytes());
        out.extend((bytes.len() as u32).to_be_bytes());
        offset += bytes.len() as u32;
    }
    out.extend(&os2);
    out.extend(&glyphs);
    out.extend(&names);
    out.extend(&post);
    out
}

fn dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("degenview-fonts-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn families(entries: &[FontEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.family.as_str()).collect()
}

#[test]
fn a_monospaced_regular_font_is_listed_by_its_family() {
    let root = dir("basic");
    fs::write(root.join("a.ttf"), font(&Spec::regular("Fira Code"), 0)).unwrap();
    let entries = scan(std::slice::from_ref(&root));
    assert_eq!(families(&entries), ["Fira Code"]);
    assert_eq!(entries[0].path, root.join("a.ttf"));
    assert_eq!(entries[0].index, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn proportional_bold_and_italic_faces_are_not_listed() {
    let root = dir("styles");
    let write = |name: &str, spec: Spec| fs::write(root.join(name), font(&spec, 0)).unwrap();
    write(
        "prop.ttf",
        Spec {
            mono: false,
            ..Spec::regular("Helvetica")
        },
    );
    write(
        "bold.ttf",
        Spec {
            subfamily: "Bold",
            weight: 700,
            selection: 0x20,
            ..Spec::regular("Onlybold Mono")
        },
    );
    write(
        "italic.ttf",
        Spec {
            subfamily: "Italic",
            selection: 0x01,
            ..Spec::regular("Onlyitalic Mono")
        },
    );
    write(
        "light.ttf",
        Spec {
            weight: 300,
            ..Spec::regular("Onlylight Mono")
        },
    );
    write("ok.ttf", Spec::regular("Good Mono"));
    assert_eq!(families(&scan(std::slice::from_ref(&root))), ["Good Mono"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_bitmap_only_font_is_not_offered_because_it_would_draw_nothing() {
    // GB18030 Bitmap, on a Mac: monospaced, regular, with every letter mapped, and no outlines.
    let root = dir("bitmap");
    let write = |name: &str, spec: Spec| fs::write(root.join(name), font(&spec, 0)).unwrap();
    write(
        "bitmap.ttf",
        Spec {
            outlines: false,
            ..Spec::regular("GB18030 Bitmap")
        },
    );
    write("ok.ttf", Spec::regular("Drawn Mono"));
    assert_eq!(families(&scan(std::slice::from_ref(&root))), ["Drawn Mono"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_font_named_only_for_the_macintosh_is_listed_too() {
    // Menlo is on every Mac and its names are Macintosh Roman only.
    let root = dir("mac-names");
    let spec = Spec {
        mac_names: true,
        ..Spec::regular("Menlo")
    };
    fs::write(root.join("menlo.ttf"), font(&spec, 0)).unwrap();
    assert_eq!(families(&scan(std::slice::from_ref(&root))), ["Menlo"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_font_is_checked_for_letters_before_the_editor_is_given_it() {
    let hack = LoadedFont {
        bytes: eframe::egui::FontDefinitions::default().font_data["Hack"]
            .font
            .to_vec(),
        index: 0,
    };
    assert_eq!(check_outlines(&hack), Ok(()), "a real font with outlines");
    // A file that is not a font, a truncated one, and one whose tables say nothing.
    for bytes in [
        b"not a font".to_vec(),
        hack.bytes[..400].to_vec(),
        font(&Spec::regular("Synthetic Mono"), 0),
    ] {
        let broken = LoadedFont { bytes, index: 0 };
        assert!(check_outlines(&broken).is_err());
    }
    // A face that is not in the file.
    let missing = LoadedFont {
        bytes: hack.bytes,
        index: 9,
    };
    assert!(check_outlines(&missing).is_err());
}

#[test]
fn a_family_is_listed_once_by_its_regular_face() {
    let root = dir("dedupe");
    let write = |name: &str, spec: Spec| fs::write(root.join(name), font(&spec, 0)).unwrap();
    write(
        "Mono-Bold.ttf",
        Spec {
            subfamily: "Bold",
            weight: 700,
            selection: 0x20,
            ..Spec::regular("Same Mono")
        },
    );
    write("Mono-Regular.ttf", Spec::regular("Same Mono"));
    write("Mono-Regular2.otf", Spec::regular("same mono"));
    let entries = scan(std::slice::from_ref(&root));
    assert_eq!(
        families(&entries),
        ["Same Mono"],
        "case does not make a second family"
    );
    assert_eq!(
        entries[0].path,
        root.join("Mono-Regular.ttf"),
        "the first file in name order"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_list_is_sorted_ignoring_case() {
    let root = dir("sorted");
    for (file, family) in [
        ("1.ttf", "zeta Mono"),
        ("2.ttf", "Alpha Mono"),
        ("3.ttf", "beta Mono"),
    ] {
        fs::write(root.join(file), font(&Spec::regular(family), 0)).unwrap();
    }
    assert_eq!(
        families(&scan(std::slice::from_ref(&root))),
        ["Alpha Mono", "beta Mono", "zeta Mono"]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hidden_system_fonts_and_nameless_ones_are_skipped() {
    let root = dir("hidden");
    fs::write(root.join("a.ttf"), font(&Spec::regular(".SF NS Mono"), 0)).unwrap();
    fs::write(root.join("b.ttf"), font(&Spec::regular("   "), 0)).unwrap();
    fs::write(root.join("c.ttf"), font(&Spec::regular("Visible Mono"), 0)).unwrap();
    assert_eq!(
        families(&scan(std::slice::from_ref(&root))),
        ["Visible Mono"]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_font_without_a_regular_style_falls_back_to_its_name() {
    // No OS/2 table: "Regular" in the style name is what says so.
    let root = dir("no-os2");
    let mut bytes = font(&Spec::regular("Bare Mono"), 0);
    // Rename the OS/2 record's tag so the reader cannot find it.
    bytes[12..16].copy_from_slice(b"XXXX");
    fs::write(root.join("a.ttf"), &bytes).unwrap();
    assert_eq!(families(&scan(std::slice::from_ref(&root))), ["Bare Mono"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_collection_lists_each_member_with_its_face_index() {
    let root = dir("collection");
    let mut file = b"ttcf".to_vec();
    file.extend(0x0001_0000u32.to_be_bytes());
    file.extend(3u32.to_be_bytes());
    // Three member offsets follow the 12-byte header, so the first font starts at 24.
    let first = font(&Spec::regular("Member One"), 24);
    let second = font(
        &Spec {
            mono: false,
            ..Spec::regular("Member Proportional")
        },
        24 + first.len() as u32,
    );
    let third = font(
        &Spec::regular("Member Three"),
        24 + first.len() as u32 + second.len() as u32,
    );
    file.extend(24u32.to_be_bytes());
    file.extend((24 + first.len() as u32).to_be_bytes());
    file.extend((24 + first.len() as u32 + second.len() as u32).to_be_bytes());
    file.extend(&first);
    file.extend(&second);
    file.extend(&third);
    fs::write(root.join("many.ttc"), file).unwrap();
    let entries = scan(std::slice::from_ref(&root));
    assert_eq!(families(&entries), ["Member One", "Member Three"]);
    assert_eq!(entries[0].index, 0);
    assert_eq!(entries[1].index, 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn files_that_are_not_fonts_never_stop_the_scan() {
    let root = dir("garbage");
    fs::write(root.join("empty.ttf"), b"").unwrap();
    fs::write(root.join("text.ttf"), b"this is not a font").unwrap();
    fs::write(
        root.join("notes.txt"),
        font(&Spec::regular("Ignored Mono"), 0),
    )
    .unwrap();
    fs::write(
        root.join("web.woff"),
        font(&Spec::regular("Ignored Too"), 0),
    )
    .unwrap();
    let good = font(&Spec::regular("Fine Mono"), 0);
    fs::write(root.join("truncated.ttf"), &good[..40]).unwrap();
    fs::write(root.join("zz.ttf"), &good).unwrap();
    // A table that claims to be enormous.
    let mut huge = good.clone();
    huge[12 + 16 + 12..12 + 16 + 16].copy_from_slice(&u32::MAX.to_be_bytes());
    fs::write(root.join("huge.ttf"), huge).unwrap();
    assert_eq!(families(&scan(std::slice::from_ref(&root))), ["Fine Mono"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn folders_are_walked_to_a_depth_and_missing_ones_are_fine() {
    let root = dir("deep");
    let deep = root.join("a/b/c/d");
    fs::create_dir_all(&deep).unwrap();
    fs::write(
        root.join("a/b/Near.ttf"),
        font(&Spec::regular("Near Mono"), 0),
    )
    .unwrap();
    fs::write(deep.join("Far.ttf"), font(&Spec::regular("Far Mono"), 0)).unwrap();
    let too_deep = deep.join("e");
    fs::create_dir_all(&too_deep).unwrap();
    fs::write(
        too_deep.join("Gone.ttf"),
        font(&Spec::regular("Gone Mono"), 0),
    )
    .unwrap();
    let entries = scan(&[root.clone(), root.join("does-not-exist")]);
    assert_eq!(families(&entries), ["Far Mono", "Near Mono"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_font_is_read_whole_and_a_huge_one_is_refused() {
    let root = dir("read");
    let bytes = font(&Spec::regular("Readable Mono"), 0);
    fs::write(root.join("a.ttf"), &bytes).unwrap();
    let entry = FontEntry {
        family: "Readable Mono".into(),
        path: root.join("a.ttf"),
        index: 3,
    };
    let loaded = read_font(&entry).unwrap();
    assert_eq!(loaded.bytes, bytes);
    assert_eq!(loaded.index, 3);
    let missing = FontEntry {
        path: root.join("gone.ttf"),
        ..entry
    };
    assert!(read_font(&missing).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_catalog_scans_in_the_background_and_answers_lookups() {
    let root = dir("catalog");
    fs::write(root.join("a.ttf"), font(&Spec::regular("Bg Mono"), 0)).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let catalog = FontCatalog::start(vec![root.clone()], move || {
        let _ = sender.send(());
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(catalog.is_ready());
    assert_eq!(families(&catalog.entries()), ["Bg Mono"]);
    assert!(catalog.find("bg mono").is_some(), "lookups ignore case");
    assert!(catalog.find("Nope").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_default_catalog_has_not_scanned() {
    assert!(FontCatalog::default().entries().is_empty());
    assert!(!FontCatalog::default().is_ready());
    assert!(FontCatalog::with_entries(Vec::new()).is_ready());
}

#[test]
fn the_system_has_font_folders() {
    assert!(!system_font_dirs().is_empty());
}

#[test]
#[ignore = "reads this machine's fonts: cargo test --lib font_catalog::tests::this_machine -- --ignored --nocapture"]
fn this_machine_s_fonts_all_draw_letters() {
    let entries = scan(&system_font_dirs());
    eprintln!("{} families: {:?}", entries.len(), families(&entries));
    assert!(
        !entries.iter().any(|entry| entry.family.contains("Bitmap")),
        "a bitmap font is offered"
    );
    for entry in &entries {
        let font = read_font(entry).unwrap();
        assert_eq!(check_outlines(&font), Ok(()), "{}", entry.family);
    }
}
