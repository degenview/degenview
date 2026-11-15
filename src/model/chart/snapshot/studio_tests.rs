use super::studio::{Area, AspectRatio, Background, Include, StudioConfig, Style};

#[test]
fn the_footer_names_each_area() {
    assert_eq!(Area::FullChart.label(), "Full chart");
    assert_eq!(Area::Portion.label(), "Selected portion");
    assert_eq!(Area::EntireLayout.label(), "Entire layout");
}

#[test]
fn the_styles_are_named_for_the_control_and_the_footer() {
    assert_eq!(Style::ALL.map(Style::label), ["Chart", "Styled"]);
    for style in Style::ALL {
        assert!(!style.tooltip().is_empty());
    }
}

#[test]
fn the_area_control_says_less_than_the_footer() {
    let words: Vec<_> = [Area::FullChart, Area::Portion, Area::EntireLayout]
        .into_iter()
        .map(|area| (area.control_label(), area.label()))
        .collect();
    assert_eq!(
        words,
        [
            ("Full chart", "Full chart"),
            ("Portion", "Selected portion"),
            ("Entire layout", "Entire layout")
        ]
    );
    for area in [Area::FullChart, Area::Portion, Area::EntireLayout] {
        assert!(!area.tooltip().is_empty());
    }
}

#[test]
fn every_size_has_its_label_tooltip_and_proportions() {
    let rows: Vec<_> = AspectRatio::ALL
        .into_iter()
        .map(|size| (size.label(), size.tooltip(), size.terms(), size.ratio()))
        .collect();
    assert_eq!(
        rows,
        [
            ("Original", "The capture's own proportions", None, None),
            ("16:9", "Wide landscape", Some((16, 9)), Some(16.0 / 9.0)),
            ("3:2", "Landscape", Some((3, 2)), Some(1.5)),
            ("1:1", "Square", Some((1, 1)), Some(1.0)),
            ("9:16", "Tall portrait", Some((9, 16)), Some(9.0 / 16.0)),
        ]
    );
}

#[test]
fn the_backgrounds_are_named_and_only_none_has_no_frame() {
    let labels: Vec<_> = Background::ALL.into_iter().map(Background::label).collect();
    assert_eq!(
        labels,
        [
            "None",
            "Dots",
            "Luminous",
            "Radiant",
            "Spectrum",
            "Illuminated"
        ]
    );
    for background in Background::ALL {
        assert_eq!(background.has_frame(), background != Background::None);
    }
}

#[test]
fn a_capture_opens_styled_at_three_to_two_with_everything_included() {
    let config = StudioConfig::new(Area::Portion);
    assert_eq!(config.area, Area::Portion);
    assert_eq!(config.style, Style::Styled);
    assert_eq!(config.effective_size(), AspectRatio::Landscape3x2);
    assert_eq!(config.background, Background::None);
    assert_eq!(
        config.include,
        Include {
            indicators: true,
            drawings: true,
            branding: true
        }
    );
    assert!(config.framing.is_default());
    assert_eq!(StudioConfig::default().area, Area::FullChart);
}

#[test]
fn size_means_nothing_in_the_chart_style() {
    let mut config = StudioConfig::new(Area::FullChart);
    config.set_style(Style::Chart);
    config.size = AspectRatio::Square;
    assert_eq!(config.effective_size(), AspectRatio::Original);
    assert!(!config.size_enabled());
    config.set_style(Style::Styled);
    assert_eq!(config.effective_size(), AspectRatio::Landscape3x2);
    assert!(config.size_enabled());
}

#[test]
fn styled_opens_at_three_to_two_and_keeps_a_size_picked_since() {
    let mut config = StudioConfig::new(Area::FullChart);
    config.set_style(Style::Chart);
    config.set_style(Style::Styled);
    assert_eq!(config.size, AspectRatio::Landscape3x2);

    config.size = AspectRatio::Portrait9x16;
    config.set_style(Style::Styled);
    assert_eq!(config.size, AspectRatio::Portrait9x16, "not a switch");

    config.set_style(Style::Chart);
    assert_eq!(config.size, AspectRatio::Portrait9x16, "kept, but unused");
    config.set_style(Style::Styled);
    assert_eq!(config.size, AspectRatio::Landscape3x2, "switched to again");
}

#[test]
fn the_mark_is_drawn_in_styled_unless_switched_off() {
    let mut config = StudioConfig::new(Area::FullChart);
    config.set_style(Style::Chart);
    assert!(!config.shows_branding(), "the Chart style has none");
    config.set_style(Style::Styled);
    assert!(config.shows_branding());
    config.include.branding = false;
    assert!(!config.shows_branding());
}

#[test]
fn the_look_is_saved_as_lowercase_words() {
    let json = serde_json::to_value((
        Style::Styled,
        AspectRatio::Wide16x9,
        Background::Illuminated,
        Include::default(),
    ))
    .expect("serialises");
    assert_eq!(
        json,
        serde_json::json!([
            "styled",
            "wide16x9",
            "illuminated",
            {"indicators": true, "drawings": true, "branding": true}
        ])
    );
    for size in AspectRatio::ALL {
        let text = serde_json::to_string(&size).expect("serialises");
        assert_eq!(serde_json::from_str::<AspectRatio>(&text).ok(), Some(size));
    }
    for background in Background::ALL {
        let text = serde_json::to_string(&background).expect("serialises");
        assert_eq!(
            serde_json::from_str::<Background>(&text).ok(),
            Some(background)
        );
    }
}

#[test]
fn a_saved_include_missing_a_part_leaves_it_on() {
    let include: Include = serde_json::from_str(r#"{"drawings": false}"#).expect("reads");
    assert_eq!(
        include,
        Include {
            indicators: true,
            drawings: false,
            branding: true
        }
    );
    let all: Include = serde_json::from_str("{}").expect("reads");
    assert_eq!(all, Include::default());
}
