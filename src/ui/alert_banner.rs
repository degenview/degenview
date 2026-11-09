//! The in-app alert banner: a capsule at the top of the window when a price alert or a
//! script alert fires, gone after six seconds or on ✕. One slot per kind, so a newer alert
//! replaces an older one rather than stacking.

// Transient: the app shell hosts this when the alert runtime is wired.
#![allow(dead_code)]

use crate::model::alert::{AlertNotificationSettings, AlertTriggerEvent};
use crate::model::alert_notification::banners_enabled;
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{self, Align2, Color32, Id, Vec2};
use std::time::Duration;

/// How long a banner stays up on its own.
pub const AUTO_DISMISS_MS: i64 = 6_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Banner {
    pub id: u64,
    /// A small first line (a script alert's script and symbol).
    pub title: Option<String>,
    pub text: String,
    pub shown_at_ms: i64,
}

impl Banner {
    fn expired(&self, now_ms: i64) -> bool {
        now_ms - self.shown_at_ms >= AUTO_DISMISS_MS
    }
}

#[derive(Default)]
pub struct AlertBannerState {
    price: Option<Banner>,
    script: Option<Banner>,
}

/// "BTC reached $70,000.00".
pub fn price_banner_text(event: &AlertTriggerEvent) -> String {
    format!(
        "{} reached {}",
        event.asset.symbol,
        event.currency.format_price(event.target)
    )
}

impl AlertBannerState {
    pub fn price(&self) -> Option<&Banner> {
        self.price.as_ref()
    }

    pub fn script(&self) -> Option<&Banner> {
        self.script.as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.price.is_none() && self.script.is_none()
    }

    /// Shows a price alert unless banners are switched off. Returns whether it was shown.
    pub fn show_price(
        &mut self,
        event: &AlertTriggerEvent,
        settings: &AlertNotificationSettings,
        now_ms: i64,
    ) -> bool {
        if !banners_enabled(settings) {
            return false;
        }
        self.price = Some(Banner {
            id: event.id,
            title: None,
            text: price_banner_text(event),
            shown_at_ms: now_ms,
        });
        true
    }

    /// Shows a script alert; an empty message reads "Alert".
    pub fn show_script(
        &mut self,
        id: u64,
        title: &str,
        message: &str,
        settings: &AlertNotificationSettings,
        now_ms: i64,
    ) -> bool {
        if !banners_enabled(settings) {
            return false;
        }
        self.script = Some(Banner {
            id,
            title: Some(title.to_owned()),
            text: if message.is_empty() {
                "Alert".to_owned()
            } else {
                message.to_owned()
            },
            shown_at_ms: now_ms,
        });
        true
    }

    pub fn dismiss_price(&mut self) {
        self.price = None;
    }

    pub fn dismiss_script(&mut self) {
        self.script = None;
    }

    /// Drops banners that have been up long enough.
    pub fn expire(&mut self, now_ms: i64) {
        if self
            .price
            .as_ref()
            .is_some_and(|banner| banner.expired(now_ms))
        {
            self.price = None;
        }
        if self
            .script
            .as_ref()
            .is_some_and(|banner| banner.expired(now_ms))
        {
            self.script = None;
        }
    }

    /// Milliseconds until the next banner expires, for scheduling a repaint.
    pub fn next_expiry_in(&self, now_ms: i64) -> Option<i64> {
        [self.price.as_ref(), self.script.as_ref()]
            .into_iter()
            .flatten()
            .map(|banner| (banner.shown_at_ms + AUTO_DISMISS_MS - now_ms).max(0))
            .min()
    }
}

/// Paints whatever banners are up, centred at the top, and expires old ones.
pub fn show(ctx: &egui::Context, state: &mut AlertBannerState, now_ms: i64) {
    state.expire(now_ms);
    if state.is_empty() {
        return;
    }
    let mut dismiss_price = false;
    let mut dismiss_script = false;
    egui::Area::new(Id::new("alert-banners"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 8.0))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            if let Some(banner) = state.price.as_ref() {
                dismiss_price = capsule(ui, Icon::BellFill, banner);
            }
            if let Some(banner) = state.script.as_ref() {
                dismiss_script = capsule(ui, Icon::BellBadge, banner);
            }
        });
    if dismiss_price {
        state.dismiss_price();
    }
    if dismiss_script {
        state.dismiss_script();
    }
    if let Some(ms) = state.next_expiry_in(now_ms) {
        ctx.request_repaint_after(Duration::from_millis(ms.max(50) as u64));
    }
}

/// One capsule; returns whether its close button was clicked.
fn capsule(ui: &mut egui::Ui, icon: Icon, banner: &Banner) -> bool {
    let mut dismissed = false;
    egui::Frame::new()
        .fill(theme::pick(
            Color32::from_rgb(31, 36, 40),
            Color32::from_rgb(250, 250, 251),
        ))
        .stroke(egui::Stroke::new(1.0, kit::primary(0.1)))
        .corner_radius(999.0)
        .shadow(egui::Shadow {
            offset: [0, 3],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(100),
        })
        .inner_margin(egui::Margin::symmetric(14, 9))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::inline(ui, icon, 14.0, kit::ORANGE);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    if let Some(title) = &banner.title {
                        ui.label(kit::text(
                            title,
                            kit::size::CAPTION + 0.5,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ));
                    }
                    ui.label(kit::text(
                        &banner.text,
                        kit::size::BODY,
                        kit::REGULAR,
                        theme::text_primary(),
                    ));
                });
                if icons::button(ui, Icon::Xmark, 18.0, theme::text_secondary(), "Dismiss")
                    .clicked()
                {
                    dismissed = true;
                }
            });
        });
    dismissed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::{AlertAsset, AlertCurrency, AlertDeliveryRecord, AlertEventOrigin};
    use crate::model::{DataSource, InstrumentId};

    fn event(id: u64) -> AlertTriggerEvent {
        AlertTriggerEvent {
            id,
            alert_id: 1,
            asset: AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            observed_value: 70_050.0,
            target: 70_000.0,
            currency: AlertCurrency::Usd,
            timestamp_ms: 0,
            quote_fingerprint: "f".into(),
            origin: AlertEventOrigin::Live,
            delivery: AlertDeliveryRecord::default(),
            candle: None,
        }
    }

    #[test]
    fn a_price_banner_names_the_level_that_was_reached() {
        assert_eq!(price_banner_text(&event(1)), "BTC reached $70,000.00");
        let mut state = AlertBannerState::default();
        assert!(state.show_price(&event(1), &AlertNotificationSettings::default(), 100));
        assert_eq!(state.price().unwrap().text, "BTC reached $70,000.00");
        assert_eq!(state.price().unwrap().id, 1);
        assert!(state.script().is_none());
    }

    #[test]
    fn banners_leave_after_six_seconds() {
        let mut state = AlertBannerState::default();
        state.show_price(&event(1), &AlertNotificationSettings::default(), 1_000);
        state.expire(6_999);
        assert!(state.price().is_some());
        state.expire(7_000);
        assert!(state.price().is_none());
        assert!(state.is_empty());
    }

    #[test]
    fn a_newer_alert_replaces_the_banner_and_restarts_the_clock() {
        let settings = AlertNotificationSettings::default();
        let mut state = AlertBannerState::default();
        state.show_price(&event(1), &settings, 0);
        state.show_price(&event(2), &settings, 5_000);
        state.expire(6_500);
        assert_eq!(
            state.price().unwrap().id,
            2,
            "the first one's deadline does not dismiss the second"
        );
        state.expire(11_000);
        assert!(state.price().is_none());
    }

    #[test]
    fn price_and_script_banners_have_their_own_slots_and_deadlines() {
        let settings = AlertNotificationSettings::default();
        let mut state = AlertBannerState::default();
        state.show_price(&event(1), &settings, 0);
        state.show_script(9, "Trend · BTCUSDT 1h", "", &settings, 3_000);
        assert_eq!(
            state.script().unwrap().text,
            "Alert",
            "an empty message still says something"
        );
        assert_eq!(
            state.script().unwrap().title.as_deref(),
            Some("Trend · BTCUSDT 1h")
        );
        state.expire(6_000);
        assert!(state.price().is_none() && state.script().is_some());
        state.dismiss_script();
        assert!(state.is_empty());
    }

    #[test]
    fn switched_off_banners_never_show() {
        let off = AlertNotificationSettings {
            in_app_banners_enabled: false,
            ..Default::default()
        };
        let master = AlertNotificationSettings {
            delivery_enabled: false,
            ..Default::default()
        };
        let mut state = AlertBannerState::default();
        for settings in [&off, &master] {
            assert!(!state.show_price(&event(1), settings, 0));
            assert!(!state.show_script(1, "t", "m", settings, 0));
        }
        assert!(state.is_empty());
        // macOS notifications being off does not hide the in-app banner.
        let no_macos = AlertNotificationSettings {
            macos_notifications_enabled: false,
            ..Default::default()
        };
        assert!(state.show_price(&event(1), &no_macos, 0));
    }

    #[test]
    fn the_next_expiry_drives_the_repaint() {
        let settings = AlertNotificationSettings::default();
        let mut state = AlertBannerState::default();
        assert_eq!(state.next_expiry_in(0), None);
        state.show_price(&event(1), &settings, 1_000);
        state.show_script(2, "t", "m", &settings, 3_000);
        assert_eq!(state.next_expiry_in(2_000), Some(5_000));
        assert_eq!(state.next_expiry_in(6_500), Some(500));
        assert_eq!(state.next_expiry_in(9_500), Some(0), "never negative");
    }

    #[test]
    fn dismissing_one_leaves_the_other() {
        let settings = AlertNotificationSettings::default();
        let mut state = AlertBannerState::default();
        state.show_price(&event(1), &settings, 0);
        state.show_script(2, "t", "m", &settings, 0);
        state.dismiss_price();
        assert!(state.price().is_none() && state.script().is_some());
    }
}
