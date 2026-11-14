//! Small building blocks shared by the Swift-styled windows, one file per widget family: the
//! macOS type scale with real SF weights (`text`), the system palette (`palette`), status pills
//! (`pill`), ticker icons (`ticker_icon`), segmented section bars (`tab_bar`), menus (`menu`),
//! buttons, fields and form controls, cards, confirmation alerts and empty states. Every widget is
//! re-exported here, so callers write `kit::push_button` whichever file draws it.

pub(in crate::ui) mod art;
pub mod bar;
mod button;
mod card;
pub mod color_picker;
mod confirm;
pub mod date_picker;
pub mod dialog;
pub mod error_notice;
mod field;
mod form;
pub mod header_controls;
pub mod icon_tabs;
pub mod keycap;
pub mod loading;
pub mod markdown_view;
mod menu;
mod page;
mod palette;
mod pill;
pub mod popover;
pub mod sheet;
pub mod source_logo;
pub mod split;
mod tab_bar;
mod text;
mod ticker_icon;
pub mod time_text;

pub use button::{
    FooterButton, cta_button, destructive_sheet_button, footer_button, hover_button, large_button,
    large_prominent_button, push_button, sheet_button, tinted_button,
};
pub use card::{CARD_RADIUS, card_frame, paint_card_badge, paint_well};
pub use confirm::{Alert, Confirm, ConfirmOutcome, Notice, alert, confirm, notice};
pub use field::{FIELD_HEIGHT, field_frame, input_frame, plain_field, search_field};
pub use form::{
    StepDirection, color_well, paint_switch, segmented, segmented_filling, stepper, stepper_arrows,
    switch,
};
pub(crate) use menu::{MENU_FRAME_MARGIN, MENU_MAX_HEIGHT, menu_width_for, menu_width_for_rows};
pub use menu::{
    MenuRow, context_popup, fitted_context_menu, fitted_context_menu_min, fitted_menu,
    fitted_menu_rows, menu_fill, menu_fit, menu_fit_rows, menu_frame, menu_heading, menu_item,
    menu_item_marked, menu_item_with_shortcut, menu_picker, menu_picker_hinted, menu_popup,
    menu_row, menu_scope, menu_separator, menu_submenu,
};
pub use page::{empty_state, footer_bar, footer_bar_with, notice_card};
pub use palette::{ACCENT, GREEN, ORANGE, RED, Tone, control, hairline, primary, window};
pub use pill::{
    chip_size, paint_chip, paint_count, paint_status_badge, status_badge, status_badge_size,
    value_chip,
};
pub use tab_bar::{TabItem, tab_bar, tab_bar_regular, tab_bar_regular_width, tab_bar_slim};
pub use text::{
    BOLD, MEDIUM, REGULAR, SEMIBOLD, digits, galley, galley_rows, galley_words, paragraph, size,
    text,
};
pub use ticker_icon::{
    cached_logo, chart_icon, chart_icon_with_label, monogram_with, round_image, source_badge,
    ticker_icon,
};
