//! The content pages: an image's picture, name and opacity (Image), and a Post card's link,
//! preview and size (Link).

use super::DrawingProperties;
use super::rows::{Page, toggle_row};
use crate::model::Geometry;
use crate::ui::chart::settings::style::number_field;
use crate::ui::icons::Icon;
use crate::ui::{kit, settings::card, theme};
use eframe::egui::{self, Color32, Id, Sense, Ui, pos2, vec2};

impl DrawingProperties {
    pub(super) fn content_page(&mut self, ui: &mut Ui) {
        let Some(Geometry::Image(image)) = self.draft.geometry().cloned() else {
            return;
        };
        let mut page = Page::new(true);
        page.section(ui, "Picture", None);
        self.preview_picture(ui, &image.asset, image.aspect(), image.clamped_opacity());
        ui.label(kit::text(
            format!("{} × {} pixels", image.pixels[0], image.pixels[1]),
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        if kit::push_button(ui, Some(Icon::ImageTool), "Replace image…", true).clicked() {
            self.replace_requested = true;
        }

        page.section(ui, "Name", None);
        let mut name = image.name.clone();
        let focused_id = Id::new("drawing-image-name");
        let focused = ui.memory(|memory| memory.has_focus(focused_id));
        kit::field_frame(focused).show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut name)
                    .id(focused_id)
                    .frame(egui::Frame::NONE)
                    .hint_text("Name this image")
                    .char_limit(crate::model::annotation::image_content::MAX_NAME_CHARS)
                    .desired_width(f32::INFINITY),
            );
        });

        page.section(ui, "Appearance", None);
        let mut opacity = image.opacity;
        let mut keep_aspect = image.keep_aspect;
        let (low, high) = crate::model::annotation::image_content::OPACITY_RANGE;
        if let Some(percent) = card::card(ui, Icon::Sliders, "Opacity (%)", None, |ui| {
            number_field(
                ui,
                Id::new("drawing-image-opacity"),
                f64::from(image.clamped_opacity() * 100.0),
                5.0,
                f64::from(low * 100.0)..=f64::from(high * 100.0),
                0,
            )
        }) {
            opacity = percent as f32 / 100.0;
        }
        toggle_row(
            ui,
            Icon::ImageTool,
            "Keep shape",
            "Resizing keeps the picture's proportions.",
            &mut keep_aspect,
        );
        let changed =
            name != image.name || opacity != image.opacity || keep_aspect != image.keep_aspect;
        if changed && let Some(Geometry::Image(draft)) = self.draft.geometry_mut() {
            draft.name = name;
            draft.opacity = opacity;
            draft.keep_aspect = keep_aspect;
        }
    }

    pub(super) fn link_page(&mut self, ui: &mut Ui) {
        let Some(Geometry::Card { card, .. }) = self.draft.geometry().cloned() else {
            return;
        };
        let words = crate::ui::drawing::content::card::CardText::of(&card);
        let mut page = Page::new(true);
        page.section(
            ui,
            crate::model::annotation::link_card::SERVICE,
            Some("The card shows what the link says, and a preview only when you fetch one."),
        );
        let url = card.link.url();
        ui.label(kit::text(
            &url,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if kit::push_button(ui, Some(Icon::ArrowUpRightSquare), "Open link", true).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url.clone()));
            }
            if kit::push_button(ui, Some(Icon::CopyDoc), "Copy link", true).clicked() {
                ui.ctx().copy_text(url.clone());
            }
        });

        page.section(ui, "Preview", None);
        let preview = match &card.meta {
            Some(_) => [
                words.title.as_str(),
                words.excerpt.as_str(),
                words.footer.as_str(),
            ]
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
            None => "No preview fetched. Choose Preview on the card's toolbar to ask for one."
                .to_owned(),
        };
        ui.label(kit::text(
            &preview,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        ));

        page.section(ui, "Size", None);
        let (low, high) = crate::model::annotation::link_card::CARD_SIZE_RANGE;
        let mut size = card.size;
        for (axis, title, id) in [
            (0, "Width", "drawing-card-width"),
            (1, "Height", "drawing-card-height"),
        ] {
            if let Some(value) = card::card(ui, Icon::Sliders, title, None, |ui| {
                number_field(
                    ui,
                    Id::new(id),
                    f64::from(card.size[axis]),
                    10.0,
                    f64::from(low[axis])..=f64::from(high[axis]),
                    0,
                )
            }) {
                size[axis] = value as f32;
            }
        }
        if size != card.size
            && let Some(Geometry::Card { card, .. }) = self.draft.geometry_mut()
        {
            card.size = size;
        }
    }

    /// The picture, fitted in a box as wide as the page, or what stands in for it.
    fn preview_picture(&mut self, ui: &mut Ui, asset: &str, aspect: f32, opacity: f32) {
        use crate::service::annotation::asset_host::{AssetImage, lookup};
        const BOX_HEIGHT: f32 = 150.0;
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, BOX_HEIGHT), Sense::hover());
        ui.painter().rect_filled(rect, 8.0, theme::surface());
        if self.preview.as_ref().is_none_or(|(held, _)| held != asset) {
            self.preview = match lookup(ui.ctx(), asset) {
                AssetImage::Ready(pixels) => Some((
                    asset.to_owned(),
                    ui.ctx().load_texture(
                        format!("drawing-picture-preview-{asset}"),
                        (*pixels).clone(),
                        egui::TextureOptions::LINEAR,
                    ),
                )),
                _ => None,
            };
        }
        let Some((_, texture)) = &self.preview else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "The picture is not available yet",
                egui::FontId::proportional(13.0),
                theme::text_secondary(),
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
            return;
        };
        let inner = rect.shrink(8.0);
        let fitted = if inner.width() / inner.height() > aspect {
            vec2(inner.height() * aspect, inner.height())
        } else {
            vec2(inner.width(), inner.width() / aspect)
        };
        ui.painter().image(
            texture.id(),
            egui::Rect::from_center_size(inner.center(), fitted),
            egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE.gamma_multiply(opacity),
        );
    }
}
