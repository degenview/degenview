#![allow(clippy::field_reassign_with_default)]
use super::*;
use crate::model::SmallPrices;
use crate::model::chart_type::{ChartType, SizeMethod};
use crate::model::synthetic::Construction;

fn styled() -> ChartAppearance {
    let mut appearance = ChartAppearance::default();
    appearance.bullish = [1, 2, 3];
    appearance.bearish = [4, 5, 6];
    appearance.axis_decimals = Some(6);
    appearance.axis_small_prices = SmallPrices::Decimals;
    appearance.canvas.background.mode = BackgroundMode::Gradient;
    appearance.canvas.background.top = Some([9, 9, 9, 255]);
    appearance.canvas.horizontal_grid.visible = false;
    appearance.canvas.margins.right = 33;
    appearance.style.chart_type = ChartType::Renko;
    appearance.style.renko.size.method = SizeMethod::Traditional;
    appearance.style.renko.size.amount = 25.0;
    appearance.style.candles.up_body = Some([7, 7, 7, 255]);
    appearance
}

fn book() -> TemplateBook {
    let mut book = TemplateBook::default();
    book.save(FIRST_USER_ID + 1, "My Dark Theme", "", &styled(), 10)
        .expect("saved");
    book
}

// MARK: Names

#[test]
fn names_are_trimmed_and_checked() {
    assert_eq!(clean_name("  My look  ").as_deref(), Ok("My look"));
    assert_eq!(clean_name("   "), Err(TemplateError::EmptyName));
    assert_eq!(clean_name(""), Err(TemplateError::EmptyName));
    assert_eq!(
        clean_name("bad\u{0007}name"),
        Err(TemplateError::ControlCharacters)
    );
    assert_eq!(
        clean_name("tab\there"),
        Err(TemplateError::ControlCharacters)
    );
    assert_eq!(
        clean_name(&"x".repeat(MAX_NAME_CHARS)).map(|n| n.len()),
        Ok(MAX_NAME_CHARS)
    );
    assert_eq!(
        clean_name(&"x".repeat(MAX_NAME_CHARS + 1)),
        Err(TemplateError::NameTooLong)
    );
    // Length is counted in characters, not bytes.
    assert!(clean_name(&"é".repeat(MAX_NAME_CHARS)).is_ok());
}

#[test]
fn descriptions_are_optional_and_bounded() {
    assert_eq!(clean_description("  "), Ok(None));
    assert_eq!(clean_description(" calm "), Ok(Some("calm".to_owned())));
    assert_eq!(
        clean_description("two\nlines"),
        Ok(Some("two\nlines".to_owned()))
    );
    assert_eq!(
        clean_description("bad\u{0000}"),
        Err(TemplateError::ControlCharacters)
    );
    assert_eq!(
        clean_description(&"y".repeat(MAX_DESCRIPTION_CHARS + 1)),
        Err(TemplateError::DescriptionTooLong)
    );
    for error in [
        TemplateError::EmptyName,
        TemplateError::NameTooLong,
        TemplateError::DescriptionTooLong,
        TemplateError::ControlCharacters,
        TemplateError::DuplicateName,
        TemplateError::BuiltIn,
        TemplateError::Missing,
    ] {
        assert!(!error.message().is_empty());
    }
}

#[test]
fn duplicate_names_are_refused_ignoring_case_and_against_the_presets() {
    let mut book = book();
    assert_eq!(
        book.save(FIRST_USER_ID + 2, "my dark THEME", "", &styled(), 11),
        Err(TemplateError::DuplicateName)
    );
    assert_eq!(
        book.save(FIRST_USER_ID + 2, "minimal", "", &styled(), 11),
        Err(TemplateError::DuplicateName)
    );
    assert_eq!(book.saved().len(), 1, "nothing was added");
}

#[test]
fn a_free_name_numbers_itself_and_respects_the_length_limit() {
    let mut book = book();
    assert_eq!(book.free_name("Fresh"), "Fresh");
    assert_eq!(book.free_name("My Dark Theme"), "My Dark Theme 2");
    book.save(FIRST_USER_ID + 2, "My Dark Theme 2", "", &styled(), 1)
        .expect("saved");
    assert_eq!(book.free_name("My Dark Theme"), "My Dark Theme 3");
    let long = "z".repeat(MAX_NAME_CHARS);
    book.save(FIRST_USER_ID + 3, &long, "", &styled(), 1)
        .expect("saved");
    let next = book.free_name(&long);
    assert!(next.chars().count() <= MAX_NAME_CHARS);
    assert!(next.ends_with(" 2"));
    assert!(clean_name(&next).is_ok());
}

// MARK: Snapshots

#[test]
fn a_template_is_a_frozen_copy_of_the_appearance_it_was_saved_from() {
    let mut chart = styled();
    let mut book = TemplateBook::default();
    book.save(FIRST_USER_ID, "Frozen", "", &chart, 5)
        .expect("saved");
    let frozen = book.saved()[0].appearance;
    chart.canvas.margins.right = 99;
    chart.bullish = [200, 0, 0];
    assert_ne!(chart, frozen);
    assert_eq!(book.saved()[0].appearance, frozen);
    assert_eq!(book.saved()[0].appearance.canvas.margins.right, 33);
    assert_eq!(book.saved()[0].appearance.bullish, [1, 2, 3]);
}

#[test]
fn templates_round_trip_through_json_and_old_rows_decode() {
    let template = book().saved()[0].clone();
    let json = serde_json::to_string(&template).expect("serialize");
    let decoded: ChartAppearanceTemplate = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, template);
    // A row with only the required fields.
    let minimal: ChartAppearanceTemplate =
        serde_json::from_str(r#"{"id":5,"name":"Old","appearance":{}}"#).expect("decode");
    assert_eq!(minimal.appearance, ChartAppearance::default());
    assert_eq!((minimal.description, minimal.created_at), (None, 0));
}

#[test]
fn a_template_holds_no_market_data_and_nothing_but_the_look() {
    let json = serde_json::to_value(&book().saved()[0]).expect("serialize");
    let keys: Vec<&str> = json
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    for key in keys {
        assert!(
            [
                "id",
                "name",
                "description",
                "appearance",
                "created_at",
                "updated_at"
            ]
            .contains(&key),
            "{key}"
        );
    }
    let appearance = json["appearance"].as_object().expect("object");
    for key in appearance.keys() {
        assert!(
            [
                "bullish",
                "bearish",
                "axis_decimals",
                "axis_small_prices",
                "style",
                "canvas"
            ]
            .contains(&key.as_str()),
            "{key}"
        );
    }
}

// MARK: Management

#[test]
fn rename_changes_the_name_and_description_and_keeps_the_look() {
    let mut book = book();
    let id = FIRST_USER_ID + 1;
    let look = book.saved()[0].appearance;
    book.rename(id, "  Renamed ", "calm", 20).expect("renamed");
    let template = &book.saved()[0];
    assert_eq!(template.name, "Renamed");
    assert_eq!(template.description.as_deref(), Some("calm"));
    assert_eq!(
        (
            template.appearance,
            template.updated_at,
            template.created_at
        ),
        (look, 20, 10)
    );
    // Keeping its own name is not a duplicate.
    book.rename(id, "renamed", "", 21)
        .expect("same name, other case");
    assert_eq!(
        book.rename(id, "Default Light", "", 22),
        Err(TemplateError::DuplicateName)
    );
    assert_eq!(book.rename(id, " ", "", 22), Err(TemplateError::EmptyName));
    assert_eq!(book.saved()[0].name, "renamed");
    assert_eq!(
        book.rename(99_999, "Nope", "", 22),
        Err(TemplateError::Missing)
    );
}

#[test]
fn updating_replaces_only_the_look_and_only_when_asked() {
    let mut book = book();
    let id = FIRST_USER_ID + 1;
    let mut changed = styled();
    changed.canvas.crosshair.width = 4;
    // Editing a chart elsewhere never touches the saved copy.
    assert_eq!(book.saved()[0].appearance.canvas.crosshair.width, 1);
    book.update_appearance(id, &changed, 50).expect("updated");
    let template = &book.saved()[0];
    assert_eq!(template.appearance.canvas.crosshair.width, 4);
    assert_eq!(
        (
            template.name.as_str(),
            template.updated_at,
            template.created_at
        ),
        ("My Dark Theme", 50, 10)
    );
    assert_eq!(
        book.update_appearance(2, &changed, 51),
        Err(TemplateError::BuiltIn)
    );
}

#[test]
fn duplicating_makes_an_editable_copy_with_a_free_name() {
    let mut book = book();
    let id = FIRST_USER_ID + 1;
    let copy = book.duplicate(id, FIRST_USER_ID + 2, 30).expect("copy");
    assert_eq!(copy, FIRST_USER_ID + 2);
    assert_eq!(book.saved()[1].name, "My Dark Theme copy");
    assert_eq!(book.saved()[1].appearance, book.saved()[0].appearance);
    let again = book.duplicate(id, FIRST_USER_ID + 3, 31).expect("copy");
    assert_eq!(
        book.saved()
            .iter()
            .find(|t| t.id == again)
            .expect("there")
            .name,
        "My Dark Theme copy 2"
    );
    assert_eq!(
        book.duplicate(404, FIRST_USER_ID + 4, 1),
        Err(TemplateError::Missing)
    );
}

#[test]
fn a_preset_can_be_duplicated_but_never_changed() {
    let mut book = TemplateBook::default();
    let preset = BUILT_INS[3];
    let copy = book.duplicate(preset.id, FIRST_USER_ID, 1).expect("copy");
    let saved = book.saved().iter().find(|t| t.id == copy).expect("saved");
    assert_eq!(saved.name, "High Contrast copy");
    assert_eq!(saved.appearance, preset.appearance());
    assert_eq!(
        saved.description, None,
        "the preset's blurb is not the copy's"
    );
    assert_eq!(
        book.rename(preset.id, "Mine", "", 1),
        Err(TemplateError::BuiltIn)
    );
    assert_eq!(book.delete(preset.id), Err(TemplateError::BuiltIn));
    assert_eq!(
        book.update_appearance(preset.id, &styled(), 1),
        Err(TemplateError::BuiltIn)
    );
}

#[test]
fn deleting_removes_only_that_template() {
    let mut book = book();
    book.duplicate(FIRST_USER_ID + 1, FIRST_USER_ID + 2, 1)
        .expect("copy");
    book.delete(FIRST_USER_ID + 2).expect("deleted");
    assert_eq!(book.saved().len(), 1);
    assert_eq!(book.saved()[0].id, FIRST_USER_ID + 1);
    assert_eq!(book.delete(FIRST_USER_ID + 2), Err(TemplateError::Missing));
}

#[test]
fn entries_list_the_presets_first_and_never_store_them() {
    let book = book();
    let entries = book.entries();
    assert_eq!(entries.len(), BUILT_INS.len() + 1);
    assert!(
        entries[..BUILT_INS.len()]
            .iter()
            .all(|entry| entry.built_in)
    );
    assert!(!entries.last().expect("an entry").built_in);
    assert_eq!(book.saved().len(), 1, "presets are not in the saved list");
    let ids: std::collections::HashSet<_> = entries.iter().map(|entry| entry.id).collect();
    assert_eq!(ids.len(), entries.len(), "ids are unique");
    assert!(BUILT_INS.iter().all(|preset| preset.id < FIRST_USER_ID));
    assert!(new_user_id() >= FIRST_USER_ID);
}

#[test]
fn every_preset_is_valid_and_distinct() {
    let looks: Vec<ChartAppearance> = BUILT_INS.iter().map(BuiltIn::appearance).collect();
    for (preset, look) in BUILT_INS.iter().zip(&looks) {
        assert!(clean_name(preset.name).is_ok());
        assert_eq!(look.canvas, look.canvas.sanitized(), "{}", preset.name);
        assert_eq!(
            look.style.chart_type,
            ChartType::Candles,
            "presets leave the chart type alone"
        );
    }
    for (index, look) in looks.iter().enumerate() {
        assert!(
            !looks[index + 1..].contains(look),
            "{}",
            BUILT_INS[index].name
        );
    }
}

// MARK: Applying

#[test]
fn appearance_and_canvas_takes_the_template_whole_but_the_axis_decimals() {
    let mut target = ChartAppearance::default();
    target.axis_decimals = Some(2);
    target.axis_small_prices = SmallPrices::ZeroCount;
    let merged = ApplyScope::AppearanceAndCanvas.merge(&target, &styled());
    assert_eq!(merged.style, styled().style);
    assert_eq!(merged.canvas, styled().canvas);
    assert_eq!((merged.bullish, merged.bearish), ([1, 2, 3], [4, 5, 6]));
    assert_eq!(
        merged.axis_decimals,
        Some(2),
        "precision follows the market"
    );
    assert_eq!(merged.axis_small_prices, SmallPrices::ZeroCount);
}

#[test]
fn colors_and_canvas_keeps_the_target_type_and_what_its_bars_are_built_from() {
    let mut target = ChartAppearance::default();
    target.style.chart_type = ChartType::Kagi;
    target.style.kagi.size.method = SizeMethod::Percent;
    target.style.kagi.size.percent = 2.5;
    target.style.renko.size.amount = 99.0;
    target.style.point_figure.reversal = 5;
    target.style.line_break.lines = 4;
    target.style.range.size.percent = 3.0;
    let template = styled();
    let merged = ApplyScope::ColorsAndCanvas.merge(&target, &template);
    assert_eq!(merged.style.chart_type, ChartType::Kagi);
    assert_eq!(merged.canvas, template.canvas);
    assert_eq!(merged.bullish, template.bullish);
    assert_eq!(merged.axis_small_prices, SmallPrices::ZeroCount);
    // The look the template carried still arrives.
    assert_eq!(merged.style.candles.up_body, Some([7, 7, 7, 255]));
    // Whatever builds the bars is the target's: no type's construction moves.
    for kind in ChartType::ALL {
        let (mut before, mut after) = (target.style, merged.style);
        before.chart_type = kind;
        after.chart_type = kind;
        assert_eq!(
            Construction::of(&before),
            Construction::of(&after),
            "{kind:?}"
        );
    }
}

#[test]
fn applying_leaves_the_target_untouched() {
    let target = ChartAppearance::default();
    let copy = target;
    let _ = ApplyScope::ColorsAndCanvas.merge(&target, &styled());
    assert_eq!(target, copy);
}

#[test]
fn scopes_have_labels_and_hints() {
    for scope in ApplyScope::ALL {
        assert!(scope.label().starts_with("Apply"));
        assert!(!scope.hint().is_empty());
    }
}

// MARK: Apply to all

use crate::model::DataSource;

#[test]
fn applying_to_all_skips_the_source_and_gives_each_chart_what_its_market_allows() {
    let charts = [
        (1, DataSource::Binance),
        (2, DataSource::Binance),
        (3, DataSource::Coinbase),
        (4, DataSource::Polymarket),
        (5, DataSource::CoinGecko),
    ];
    let look = styled();
    let plan = plan_apply_to_all(&charts, &look, ApplyScope::AppearanceAndCanvas, 1);
    let ids: Vec<u64> = plan.steps.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, [2, 3, 4, 5], "the source keeps what it has");
    let scope_of = |id| {
        plan.steps
            .iter()
            .find(|(chart, _)| *chart == id)
            .expect("planned")
            .1
    };
    assert_eq!(scope_of(2), ApplyScope::AppearanceAndCanvas);
    // A prediction market is a line: it never takes a chart type, and is not reported for it.
    assert_eq!(scope_of(4), ApplyScope::ColorsAndCanvas);
    // Any market that cannot draw the look's type keeps its own and is counted.
    for (id, market) in charts.iter().skip(1) {
        let takes = ChartType::Renko.supported_by(*market) && !market.is_line_chart();
        assert_eq!(
            scope_of(*id) == ApplyScope::AppearanceAndCanvas,
            takes,
            "{market:?}"
        );
    }
    assert_eq!(
        plan.kept_type,
        charts
            .iter()
            .skip(1)
            .filter(|(_, market)| !market.is_line_chart() && !ChartType::Renko.supported_by(*market))
            .count()
    );
}

#[test]
fn applying_colors_only_to_all_never_reports_a_kept_type() {
    let charts = [(1, DataSource::Binance), (2, DataSource::Coinbase)];
    let plan = plan_apply_to_all(&charts, &styled(), ApplyScope::ColorsAndCanvas, 1);
    assert_eq!(plan.steps, vec![(2, ApplyScope::ColorsAndCanvas)]);
    assert_eq!(plan.kept_type, 0);
}

#[test]
fn the_apply_to_all_message_counts_and_explains() {
    let none = plan_apply_to_all(
        &[(1, DataSource::Binance)],
        &styled(),
        ApplyScope::AppearanceAndCanvas,
        1,
    );
    assert_eq!(
        none.message("Renko"),
        "There are no other charts in this workspace."
    );
    let one = ApplyPlan {
        steps: vec![(2, ApplyScope::AppearanceAndCanvas)],
        kept_type: 0,
    };
    assert_eq!(one.message("Renko"), "Applied the appearance to 1 chart.");
    let skipped = ApplyPlan {
        steps: vec![
            (2, ApplyScope::ColorsAndCanvas),
            (3, ApplyScope::AppearanceAndCanvas),
        ],
        kept_type: 1,
    };
    let text = skipped.message("Renko");
    assert!(
        text.starts_with("Applied the appearance to 2 charts."),
        "{text}"
    );
    assert!(
        text.contains("1 kept its chart type") && text.contains("cannot draw Renko"),
        "{text}"
    );
    assert_eq!(skipped.steps.len(), 2);
}

#[test]
fn colours_and_canvas_keep_what_a_footprint_is_built_from_and_take_the_look() {
    let mut target = ChartAppearance::default();
    target.style.chart_type = ChartType::VolumeFootprint;
    target.style.footprint.ticks_per_row = 25;
    target.style.footprint.imbalance_percent = 450;
    let mut template = ChartAppearance::default();
    template.style.footprint.ticks_per_row = 3;
    template.style.footprint.imbalance_percent = 200;
    template.style.footprint.buy_color = Some([5, 6, 7, 255]);
    let merged = ApplyScope::ColorsAndCanvas.merge(&target, &template);
    assert_eq!(merged.style.chart_type, ChartType::VolumeFootprint);
    assert_eq!(merged.style.footprint.ticks_per_row, 25);
    assert_eq!(merged.style.footprint.imbalance_percent, 450);
    assert_eq!(merged.style.footprint.buy_color, Some([5, 6, 7, 255]));
}

#[test]
fn colours_and_canvas_keep_what_a_tpo_is_built_from_and_take_the_look() {
    let mut target = ChartAppearance::default();
    target.style.chart_type = ChartType::TimePriceOpportunity;
    target.style.tpo.block_minutes = 60;
    target.style.tpo.value_area_percent = 80;
    let mut template = ChartAppearance::default();
    template.style.tpo.block_minutes = 5;
    template.style.tpo.value_area_percent = 50;
    template.style.tpo.gradient[2] = Some([9, 8, 7, 255]);
    let merged = ApplyScope::ColorsAndCanvas.merge(&target, &template);
    assert_eq!(merged.style.tpo.block_minutes, 60);
    assert_eq!(merged.style.tpo.value_area_percent, 80);
    assert_eq!(merged.style.tpo.gradient[2], Some([9, 8, 7, 255]));
}
