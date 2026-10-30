use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};

/// The app theme, as Settings ▸ Appearance stores it under [`THEME_KEY`] (Swift's `appTheme`).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AppTheme {
    System,
    Light,
    #[default]
    Dark,
}

impl AppTheme {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn title(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }
}

pub const THEME_KEY: &str = "appTheme";

/// The neutral colours that change with the theme. Accents and chart semantics (up, down,
/// replay…) stay the same in both.
#[derive(Clone, Copy, Debug)]
struct Palette {
    background: Color32,
    chrome: Color32,
    surface: Color32,
    surface_selected: Color32,
    border: Color32,
    grid: Color32,
    text_primary: Color32,
    text_secondary: Color32,
    text_tertiary: Color32,
    separator: Color32,
    editor_background: Color32,
    /// `labelColor` itself, for translucent fills: white on dark, black on light.
    ink: Color32,
}

const DARK: Palette = Palette {
    background: Color32::from_rgb(28, 36, 40),
    chrome: Color32::from_rgb(37, 45, 49),
    surface: Color32::from_rgb(31, 40, 44),
    surface_selected: Color32::from_rgb(45, 55, 60),
    border: Color32::from_rgb(57, 68, 73),
    grid: Color32::from_rgb(48, 59, 64),
    text_primary: Color32::from_rgb(235, 235, 240),
    text_secondary: Color32::from_rgb(152, 152, 157),
    text_tertiary: Color32::from_rgb(98, 98, 102),
    separator: Color32::from_rgb(58, 69, 74),
    editor_background: Color32::from_rgb(24, 31, 34),
    ink: Color32::WHITE,
};

/// macOS's light system colours: `windowBackgroundColor`, white content, `labelColor` at the
/// secondary/tertiary opacities over white.
const LIGHT: Palette = Palette {
    background: Color32::from_rgb(246, 246, 247),
    chrome: Color32::from_rgb(236, 236, 238),
    surface: Color32::from_rgb(255, 255, 255),
    surface_selected: Color32::from_rgb(226, 228, 232),
    border: Color32::from_rgb(208, 209, 213),
    grid: Color32::from_rgb(236, 237, 240),
    text_primary: Color32::from_rgb(29, 29, 31),
    text_secondary: Color32::from_rgb(110, 110, 115),
    text_tertiary: Color32::from_rgb(170, 170, 175),
    separator: Color32::from_rgb(218, 219, 222),
    editor_background: Color32::from_rgb(255, 255, 255),
    ink: Color32::BLACK,
};

/// The two cells that say which theme is showing: whether the light palette is active (set from
/// the context's resolved theme every frame, so System follows the Mac as it switches), and the
/// theme picked in Settings as an index into [`AppTheme::ALL`] (starting at the default, Dark).
///
/// In the app they are process-wide. Under `cfg(test)` every test thread has its own pair, so a
/// test that switches to the light theme can never be seen by another that renders, compares
/// pixels or checks the choice while it runs; they all start from Dark.
#[cfg(not(test))]
mod state {
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

    static LIGHT_ACTIVE: AtomicBool = AtomicBool::new(false);
    static CHOICE: AtomicU8 = AtomicU8::new(super::DEFAULT_CHOICE);

    pub fn light() -> bool {
        LIGHT_ACTIVE.load(Ordering::Relaxed)
    }
    pub fn set_light(light: bool) {
        LIGHT_ACTIVE.store(light, Ordering::Relaxed);
    }
    pub fn choice() -> u8 {
        CHOICE.load(Ordering::Relaxed)
    }
    pub fn set_choice(choice: u8) {
        CHOICE.store(choice, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod state {
    use std::cell::Cell;

    thread_local! {
        static LIGHT_ACTIVE: Cell<bool> = const { Cell::new(false) };
        static CHOICE: Cell<u8> = const { Cell::new(super::DEFAULT_CHOICE) };
    }

    pub fn light() -> bool {
        LIGHT_ACTIVE.with(Cell::get)
    }
    pub fn set_light(light: bool) {
        LIGHT_ACTIVE.with(|cell| cell.set(light));
    }
    pub fn choice() -> u8 {
        CHOICE.with(Cell::get)
    }
    pub fn set_choice(choice: u8) {
        CHOICE.with(|cell| cell.set(choice));
    }
}

/// The index of Dark, the default, in [`AppTheme::ALL`].
const DEFAULT_CHOICE: u8 = 2;

/// Puts the theme back to the default when dropped, for a test that switches it: libtest may run
/// several tests on one thread (`--test-threads=1`), and the next must not inherit Light.
#[cfg(test)]
pub(crate) struct RestoreDefaultTheme;

#[cfg(test)]
impl Drop for RestoreDefaultTheme {
    fn drop(&mut self) {
        state::set_light(false);
        state::set_choice(DEFAULT_CHOICE);
    }
}

/// The theme picked in Settings ▸ Appearance.
pub fn choice() -> AppTheme {
    AppTheme::ALL
        .get(state::choice() as usize)
        .copied()
        .unwrap_or_default()
}

fn palette() -> &'static Palette {
    if is_light() { &LIGHT } else { &DARK }
}

pub fn is_light() -> bool {
    state::light()
}

/// Picks `dark` or `light` for the active theme.
pub fn pick(dark: Color32, light: Color32) -> Color32 {
    if is_light() { light } else { dark }
}

/// Picks `dark` or `light` for the active theme, for a plain number.
pub fn pick_value(dark: f32, light: f32) -> f32 {
    if is_light() { light } else { dark }
}

pub fn background() -> Color32 {
    palette().background
}
pub fn chrome() -> Color32 {
    palette().chrome
}
pub fn surface() -> Color32 {
    palette().surface
}
pub fn surface_selected() -> Color32 {
    palette().surface_selected
}
pub fn border() -> Color32 {
    palette().border
}
pub fn grid() -> Color32 {
    palette().grid
}
/// `labelColor`.
pub fn text_primary() -> Color32 {
    palette().text_primary
}
/// `Color.secondary` / `secondaryLabelColor`.
pub fn text_secondary() -> Color32 {
    palette().text_secondary
}
/// `tertiaryLabelColor`.
pub fn text_tertiary() -> Color32 {
    palette().text_tertiary
}
/// `NSColor.separatorColor`.
pub fn separator() -> Color32 {
    palette().separator
}
/// `NSColor.textBackgroundColor`: the editor surface.
pub fn editor_background() -> Color32 {
    palette().editor_background
}
/// The raised plate under a selected segment: a lifted grey on dark, white on light.
pub fn selected_plate() -> Color32 {
    pick(Color32::from_rgb(88, 98, 104), Color32::WHITE)
}
/// Text and glyphs on [`selected_plate`].
pub fn on_selected_plate() -> Color32 {
    pick(Color32::WHITE, palette().text_primary)
}
/// A bordered push button or stepper's bezel (`controlColor`).
pub fn control_fill() -> Color32 {
    pick(Color32::from_rgb(63, 72, 77), Color32::WHITE)
}
/// `NSColor.windowBackgroundColor`: the gutter.
pub fn gutter_background() -> Color32 {
    palette().background
}

/// The editor's unsaved-change marks: a bar for added lines, a wedge for deleted ones, a blue bar
/// for modified ones. Muted so they sit beside the code rather than over it, and each pair keeps
/// its contrast on the gutter in both appearances.
pub fn editor_diff_added() -> Color32 {
    pick(
        Color32::from_rgb(63, 185, 96),
        Color32::from_rgb(30, 140, 62),
    )
}
pub fn editor_diff_deleted() -> Color32 {
    pick(
        Color32::from_rgb(236, 90, 82),
        Color32::from_rgb(207, 34, 46),
    )
}
pub fn editor_diff_modified() -> Color32 {
    pick(
        Color32::from_rgb(76, 156, 255),
        Color32::from_rgb(0, 102, 214),
    )
}
/// Row tints in the diff preview: green for current lines, red for saved ones (the modified
/// colour marks the gutter only; a modified hunk's two sides are still saved/current).
pub fn editor_diff_added_background() -> Color32 {
    with_alpha(editor_diff_added(), pick_alpha(0.16, 0.14))
}
pub fn editor_diff_deleted_background() -> Color32 {
    with_alpha(editor_diff_deleted(), pick_alpha(0.16, 0.12))
}
/// The debugger's marks in the editor. A breakpoint is red, as everywhere; one that is switched off
/// or cannot resolve is muted. The line the script is stopped on is amber, a caller's line (picked in
/// the call stack) the same at a lower strength, and a failure red.
pub fn editor_breakpoint() -> Color32 {
    pick(
        Color32::from_rgb(236, 90, 82),
        Color32::from_rgb(207, 34, 46),
    )
}
pub fn editor_breakpoint_muted() -> Color32 {
    pick(
        Color32::from_rgb(132, 134, 142),
        Color32::from_rgb(150, 152, 160),
    )
}
pub fn editor_debug_line() -> Color32 {
    pick(
        Color32::from_rgb(255, 196, 41),
        Color32::from_rgb(184, 124, 0),
    )
}
pub fn editor_debug_line_background() -> Color32 {
    with_alpha(editor_debug_line(), pick_alpha(0.17, 0.2))
}
pub fn editor_debug_caller_background() -> Color32 {
    with_alpha(editor_debug_line(), pick_alpha(0.08, 0.1))
}
pub fn editor_debug_error_background() -> Color32 {
    with_alpha(editor_breakpoint(), pick_alpha(0.2, 0.14))
}
/// The profiler's marks in the editor. A bar runs from a cool slate, for a line that does little
/// itself, to a warm orange, for one where the script's time goes; the hovered code block is a faint
/// wash of the warm end, and a flame marks the hottest lines.
pub fn profiler_heat(heat: f32) -> Color32 {
    let cool = pick(
        Color32::from_rgb(112, 124, 146),
        Color32::from_rgb(138, 150, 170),
    );
    let warm = pick(
        Color32::from_rgb(255, 138, 48),
        Color32::from_rgb(222, 96, 12),
    );
    cool.lerp_to_gamma(warm, heat.clamp(0.0, 1.0))
}
/// A caution in the profiler's words (a stale or partial profile): amber on dark, a darker amber on
/// light, where yellow text cannot be read.
pub fn profiler_caution() -> Color32 {
    pick(YELLOW, Color32::from_rgb(166, 104, 0))
}
pub fn profiler_track() -> Color32 {
    ink(pick_alpha_byte(14, 16))
}
pub fn profiler_block() -> Color32 {
    with_alpha(profiler_heat(1.0), pick_alpha(0.10, 0.09))
}
pub fn profiler_flame() -> Color32 {
    profiler_heat(1.0)
}
fn pick_alpha_byte(dark: u8, light: u8) -> u8 {
    if is_light() { light } else { dark }
}
fn pick_alpha(dark: f32, light: f32) -> f32 {
    if is_light() { light } else { dark }
}

pub const UP: Color32 = Color32::from_rgb(23, 201, 114);
pub const DOWN: Color32 = Color32::from_rgb(245, 72, 76);
pub const ACCENT: Color32 = Color32::from_rgb(191, 82, 203);
pub const RSI: Color32 = Color32::from_rgb(160, 130, 255);
pub const EMA: Color32 = Color32::from_rgb(255, 165, 0);
pub const BOLLINGER: Color32 = Color32::from_rgb(0, 200, 255);
/// Marks a time-travelled chart: replay bar, plot border and badge.
pub const REPLAY: Color32 = Color32::from_rgb(255, 149, 0);

pub fn install(ctx: &egui::Context, theme: AppTheme) {
    install_styles(ctx);
    apply(ctx, theme);
}

/// Fonts, spacing and colours for a context that only draws, a snapshot's. It follows the theme
/// the app shows now and, unlike [`install`], never changes the app's own palette.
pub fn install_offscreen(ctx: &egui::Context) {
    install_styles(ctx);
    ctx.set_theme(if is_light() {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });
}

fn install_styles(ctx: &egui::Context) {
    install_fonts(ctx);
    for (egui_theme, palette) in [(egui::Theme::Dark, &DARK), (egui::Theme::Light, &LIGHT)] {
        ctx.set_visuals_of(egui_theme, visuals(egui_theme, palette));
        ctx.style_mut_of(egui_theme, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(9.0, 5.0);
            style.spacing.interact_size.y = 26.0;
            style.visuals.widgets.inactive.corner_radius = 6.0.into();
            style.visuals.widgets.hovered.corner_radius = 6.0.into();
            style.visuals.widgets.active.corner_radius = 6.0.into();
        });
    }
}

/// Switches to `theme`; System follows the Mac's appearance.
pub fn apply(ctx: &egui::Context, theme: AppTheme) {
    let index = AppTheme::ALL.iter().position(|each| *each == theme);
    state::set_choice(index.unwrap_or_default() as u8);
    ctx.set_theme(theme.preference());
    sync(ctx);
}

/// Points the palette at the context's resolved theme. Called at the top of every frame.
pub fn sync(ctx: &egui::Context) {
    state::set_light(ctx.theme() == egui::Theme::Light);
}

fn visuals(theme: egui::Theme, palette: &Palette) -> egui::Visuals {
    let light = theme == egui::Theme::Light;
    let mut visuals = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let shade = |dark: (u8, u8, u8), light_rgb: (u8, u8, u8)| {
        let (r, g, b) = if light { light_rgb } else { dark };
        Color32::from_rgb(r, g, b)
    };
    visuals.override_text_color = Some(palette.text_primary);
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.chrome;
    visuals.window_stroke = egui::Stroke::new(1.0, palette.border);
    visuals.extreme_bg_color = palette.editor_background;
    visuals.faint_bg_color = palette.surface;
    visuals.selection.bg_fill = if light {
        ACCENT.gamma_multiply(0.35)
    } else {
        ACCENT
    };
    visuals.selection.stroke.color = if light { ACCENT } else { Color32::WHITE };
    visuals.hyperlink_color = SYSTEM_ACCENT;
    visuals.widgets.noninteractive.bg_fill = palette.surface;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, palette.border);
    visuals.widgets.noninteractive.fg_stroke.color = palette.text_primary;
    visuals.widgets.inactive.bg_fill = shade((51, 61, 66), (228, 229, 232));
    visuals.widgets.inactive.weak_bg_fill = shade((51, 61, 66), (228, 229, 232));
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, palette.border);
    visuals.widgets.inactive.fg_stroke.color = palette.text_primary;
    visuals.widgets.hovered.bg_fill = shade((65, 76, 81), (218, 219, 223));
    visuals.widgets.hovered.weak_bg_fill = shade((65, 76, 81), (218, 219, 223));
    visuals.widgets.hovered.fg_stroke.color = palette.text_primary;
    visuals.widgets.active.bg_fill = shade((77, 88, 94), (206, 208, 212));
    visuals.widgets.active.weak_bg_fill = shade((77, 88, 94), (206, 208, 212));
    visuals.widgets.active.fg_stroke.color = palette.text_primary;
    visuals.window_corner_radius = 8.0.into();
    visuals
}

// MARK: Script Manager tokens (Swift dark appearance: macOS system colours, quaternary fills)

pub const YELLOW: Color32 = Color32::from_rgb(255, 214, 10);
pub const ORANGE: Color32 = Color32::from_rgb(255, 159, 10);
pub const WIN: Color32 = Color32::from_rgb(0x26, 0xA6, 0x9A);
pub const LOSS: Color32 = Color32::from_rgb(0xEF, 0x53, 0x50);

/// The label colour at `alpha` (0–255): white on dark, black on light. The `.quaternary`
/// family of fills and hover washes.
pub fn ink(alpha: u8) -> Color32 {
    let ink = palette().ink;
    Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), alpha)
}

/// `Color.quaternary` ≈ 0.18, `.quaternary.opacity(0.45)` ≈ 0.08 of the label colour (light
/// windows need a touch less to read the same).
pub fn quaternary(opacity: f32) -> Color32 {
    let full = if is_light() { 30.0 } else { 46.0 };
    ink((full * opacity.clamp(0.0, 1.0)).round() as u8)
}

/// The fill of a settings card: a few alpha steps of the ink, so it sits on `chrome` and on
/// `background` alike. Edges and a top highlight define it more than the fill does.
pub fn card_fill(hovered: bool) -> Color32 {
    let alpha = match (is_light(), hovered) {
        (false, false) => 11,
        (false, true) => 15,
        (true, false) => 7,
        (true, true) => 11,
    };
    ink(alpha)
}

/// The hairline around a settings card; it lifts a little under the pointer.
pub fn card_border(hovered: bool) -> Color32 {
    ink(if hovered { 24 } else { 14 })
}

/// The 1px line along a card's top edge that reads as light falling on it from above.
pub fn card_highlight() -> Color32 {
    pick(
        Color32::from_white_alpha(26),
        Color32::from_white_alpha(200),
    )
}

/// An inset well, darker than the card it sits on: the off track of a switch.
pub fn well_fill() -> Color32 {
    pick(Color32::from_black_alpha(70), ink(26))
}

pub fn with_alpha(color: Color32, opacity: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (255.0 * opacity.clamp(0.0, 1.0)).round() as u8,
    )
}

/// The macOS system colours the app uses for code-like values outside the editor: sample code in
/// empty states, the debugger's value types, status text. The editor's own colours are its theme's
/// (`ui::editor_appearance`).
pub mod syntax {
    use super::{Color32, pick};

    macro_rules! system {
        ($($(#[$doc:meta])* $name:ident: $dark:expr, $light:expr;)*) => {$(
            $(#[$doc])*
            pub fn $name() -> Color32 {
                let (dark, light): ((u8, u8, u8), (u8, u8, u8)) = ($dark, $light);
                pick(
                    Color32::from_rgb(dark.0, dark.1, dark.2),
                    Color32::from_rgb(light.0, light.1, light.2),
                )
            }
        )*};
    }

    system! {
        /// `systemPurple`.
        keyword: (0xBF, 0x5A, 0xF2), (0xAF, 0x52, 0xDE);
        /// `systemBlue`.
        function: (0x0A, 0x84, 0xFF), (0x00, 0x7A, 0xFF);
        /// `systemGreen`.
        variable: (0x30, 0xD1, 0x58), (0x28, 0xA7, 0x45);
        /// `systemOrange`.
        number: (0xFF, 0x9F, 0x0A), (0xE8, 0x80, 0x00);
        /// `systemRed`.
        string: (0xFF, 0x45, 0x3A), (0xFF, 0x3B, 0x30);
    }
}

/// macOS's default system accent (`Color.accentColor`), which the Swift Script Manager uses for
/// selection, tabs and links. The rest of the Rust app keeps its own `ACCENT`.
pub const SYSTEM_ACCENT: Color32 = Color32::from_rgb(10, 132, 255);

/// Puts the macOS system fonts (SF Pro for text, SF Mono for code) ahead of egui's bundled ones,
/// like the Swift app, which uses them everywhere. The bundled fonts stay as glyph fallbacks, and
/// a machine without the files simply keeps the defaults. STIX Two Math and Apple Symbols go last
/// in both families so characters the others lack — "𝓑𝓾𝔂" and other math alphanumerics, arrows,
/// geometric shapes — draw instead of showing tofu squares.
fn base_fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for (name, path, family) in [
        (
            "sf-pro",
            "/System/Library/Fonts/SFNS.ttf",
            egui::FontFamily::Proportional,
        ),
        (
            "sf-mono",
            "/System/Library/Fonts/SFNSMono.ttf",
            egui::FontFamily::Monospace,
        ),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                name.to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, name.to_owned());
        }
    }
    for (name, path) in [
        (
            "stix-math",
            "/System/Library/Fonts/Supplemental/STIXTwoMath.otf",
        ),
        ("apple-symbols", "/System/Library/Fonts/Apple Symbols.ttf"),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                name.to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push(name.to_owned());
            }
        }
    }
    add_symbol_fallback(&mut fonts);
    fonts
}

/// The app's fonts without the editor's choice, read from the system once.
fn system_fonts() -> &'static egui::FontDefinitions {
    static FONTS: std::sync::OnceLock<egui::FontDefinitions> = std::sync::OnceLock::new();
    FONTS.get_or_init(base_fonts)
}

/// What the editor's code family starts with.
#[derive(Clone)]
pub enum EditorFontSource {
    /// The system's monospace font alone.
    System,
    /// The font that ships with the app: JetBrains Mono, the default.
    Bundled,
    /// A font a person picked, read from its file.
    File(std::sync::Arc<egui::FontData>),
}

/// JetBrains Mono NL Regular (`assets/fonts`, SIL OFL 1.1; the licence and where the file comes from
/// are in `assets/fonts/README.md`): the font of the editor's code unless a person picks another. The
/// NL build has no ligatures, which the editor could not switch on or off.
pub fn bundled_editor_font() -> std::sync::Arc<egui::FontData> {
    static FONT: std::sync::OnceLock<std::sync::Arc<egui::FontData>> = std::sync::OnceLock::new();
    const JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMonoNL-Regular.ttf");
    FONT.get_or_init(|| std::sync::Arc::new(egui::FontData::from_static(JETBRAINS_MONO)))
        .clone()
}

/// The app's fonts plus the family the editor's code is set in: the editor's font first, then the
/// monospace font and its fallbacks, so a character the editor's font lacks (an arrow, a symbol) still
/// draws. The family is always defined, so naming it can never fail.
fn font_definitions(editor: &EditorFontSource) -> egui::FontDefinitions {
    let mut fonts = system_fonts().clone();
    let mut chain = fonts
        .families
        .get(&egui::FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    let data = match editor {
        EditorFontSource::System => None,
        EditorFontSource::Bundled => Some(bundled_editor_font()),
        EditorFontSource::File(data) => Some(data.clone()),
    };
    if let Some(data) = data {
        // A font a person picked comes first and the built-in one next, so a character it lacks
        // draws in the font the editor is meant to look like before the system's fallbacks.
        if matches!(editor, EditorFontSource::File(_)) {
            fonts
                .font_data
                .insert(EDITOR_FALLBACK.to_owned(), bundled_editor_font());
            chain.insert(0, EDITOR_FALLBACK.to_owned());
        }
        fonts.font_data.insert(EDITOR_FONT.to_owned(), data);
        chain.insert(0, EDITOR_FONT.to_owned());
    }
    fonts
        .families
        .insert(egui::FontFamily::Name(EDITOR_FAMILY.into()), chain);
    fonts
}

const EDITOR_FONT: &str = "editor-code-font";
const EDITOR_FALLBACK: &str = "editor-code-fallback";
use crate::ui::editor_appearance::EDITOR_FAMILY;

/// The app's fonts with the bundled editor font, which is what the editor is in until the host says
/// otherwise: so a context that never hears from the host (a snapshot, a test) draws the default.
fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions(&EditorFontSource::Bundled));
}

/// Sets the font the editor's code is drawn in. egui compares the new fonts to the old byte for byte, so
/// this is called when the choice changes, not every frame.
pub fn install_editor_font(ctx: &egui::Context, font: &EditorFontSource) {
    ctx.set_fonts(font_definitions(font));
}

/// A subset of Noto Sans Symbols 2 (`assets/fonts`), the last fallback in both families. Neither
/// egui's bundled fonts nor a Linux or Windows install can be counted on to have "▲", "✔" or "★",
/// which a Pine `plotchar`, a label or a text drawing may use.
fn add_symbol_fallback(fonts: &mut egui::FontDefinitions) {
    const SYMBOLS: &[u8] = include_bytes!("../../assets/fonts/NotoSansSymbols2-subset.ttf");
    fonts.font_data.insert(
        "noto-symbols".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(SYMBOLS)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("noto-symbols".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On egui's own fonts alone, with nothing from the system, as on a bare Linux install.
    #[test]
    fn the_bundled_symbols_draw_without_any_system_font() {
        let draws = |with_fallback: bool| {
            let mut fonts = egui::FontDefinitions::default();
            if with_fallback {
                add_symbol_fallback(&mut fonts);
            }
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts);
            let mut output = ctx.run_ui(Default::default(), |_| {});
            output.textures_delta.clear();
            let mut missing = Vec::new();
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                let font = egui::FontId::new(14.0, family);
                let tofu = ctx.fonts_mut(|fonts| fonts.glyph_width(&font, '\u{10FFFF}'));
                for symbol in "▲▼●■◆★☆✔✖⬆⬇⚠☑".chars() {
                    let width = ctx.fonts_mut(|fonts| fonts.glyph_width(&font, symbol));
                    if width <= 0.0 || width == tofu {
                        missing.push(symbol);
                    }
                }
            }
            missing
        };
        assert!(
            !draws(false).is_empty(),
            "egui's own fonts now have every symbol: the bundled font may be dropped"
        );
        assert_eq!(draws(true), Vec::<char>::new());
    }

    #[test]
    fn math_alphanumerics_have_a_fallback_glyph_in_both_families() {
        if !std::path::Path::new("/System/Library/Fonts/Supplemental/STIXTwoMath.otf").exists() {
            return;
        }
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let font = egui::FontId::new(14.0, family);
            let width = ctx.fonts_mut(|fonts| fonts.glyph_width(&font, '𝓑'));
            let missing = ctx.fonts_mut(|fonts| fonts.glyph_width(&font, '\u{10FFFF}'));
            assert!(width > 0.0 && width != missing);
        }
    }

    /// A tiny price's zero count (`0.0₅278`) is drawn with these, in the monospace price font
    /// (SF Mono has none, so a fallback must) and in the proportional one.
    #[test]
    fn the_zero_count_subscripts_have_a_glyph_in_both_families() {
        if !std::path::Path::new("/System/Library/Fonts/SFNSMono.ttf").exists() {
            return;
        }
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let font = egui::FontId::new(14.0, family);
            let missing = ctx.fonts_mut(|fonts| fonts.glyph_width(&font, '\u{10FFFF}'));
            for digit in '₀'..='₉' {
                let width = ctx.fonts_mut(|fonts| fonts.glyph_width(&font, digit));
                assert!(width > 0.0 && width != missing, "{digit} in {font:?}");
            }
        }
    }

    /// Holds in either theme, so it does not depend on which one a parallel test left active.
    #[test]
    fn a_hovered_card_is_lifted_above_a_resting_one() {
        assert!(card_fill(true).a() > card_fill(false).a());
        assert!(card_border(true).a() > card_border(false).a());
        assert!(card_fill(false).a() > 0 && card_highlight().a() > 0);
    }

    #[test]
    fn the_theme_round_trips_under_the_swift_key_and_names() {
        assert_eq!(THEME_KEY, "appTheme");
        for (theme, raw) in AppTheme::ALL
            .into_iter()
            .zip(["\"System\"", "\"Light\"", "\"Dark\""])
        {
            assert_eq!(serde_json::to_string(&theme).unwrap(), raw);
            assert_eq!(serde_json::from_str::<AppTheme>(raw).unwrap(), theme);
        }
        assert_eq!(AppTheme::default(), AppTheme::Dark);
        assert_eq!(AppTheme::ALL[state::choice() as usize], AppTheme::default());
    }

    #[test]
    fn the_palette_follows_the_resolved_theme() {
        let _theme = RestoreDefaultTheme;
        let ctx = egui::Context::default();
        install(&ctx, AppTheme::Light);
        assert!(is_light());
        assert_eq!(choice(), AppTheme::Light);
        assert_eq!(surface(), LIGHT.surface);
        assert_eq!(ink(255), Color32::BLACK);
        apply(&ctx, AppTheme::Dark);
        assert!(!is_light());
        assert_eq!(choice(), AppTheme::Dark);
        assert_eq!(text_primary(), DARK.text_primary);
        assert_eq!(ink(255), Color32::WHITE);
    }
}
