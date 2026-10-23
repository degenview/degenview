//! The card a Post drawing paints on the chart: native shapes and text, no web view. It shows what
//! the link proves (the service, an X handle, an id, the post's time) and, only once the user asked
//! for a preview, what that returned. Nothing else
//! is ever shown: no avatar, counts or text are made up to fill the card.

use crate::format;
use crate::model::link_card::{LinkCard, SERVICE};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, FontId, Painter, Rect, Stroke, StrokeKind, text::LayoutJob, vec2,
};

const PADDING: f32 = 10.0;
const CORNER: f32 = 8.0;
const HEADER_HEIGHT: f32 = 20.0;

/// The words of a card, before they are laid out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardText {
    pub service: &'static str,
    /// The handle of the post.
    pub headline: String,
    pub title: String,
    pub excerpt: String,
    pub footer: String,
}

impl CardText {
    pub fn of(card: &LinkCard) -> Self {
        let link = &card.link;
        let meta = card.meta.as_ref();
        let fetched_title = meta
            .map(|meta| meta.title.clone())
            .filter(|t| !t.is_empty());
        let title = fetched_title.unwrap_or_else(|| format!("Post {}", link.id));
        let mut footer = Vec::new();
        match meta {
            Some(meta) => {
                if !meta.author.is_empty() {
                    footer.push(meta.author.clone());
                }
            }
            None => footer.push("Preview not fetched".to_owned()),
        }
        if let Some(time) = link.time_ms() {
            footer.push(format::timestamp(time));
        }
        Self {
            service: SERVICE,
            headline: link.headline(),
            title,
            excerpt: meta.map(|meta| meta.excerpt.clone()).unwrap_or_default(),
            footer: footer.join(" · "),
        }
    }
}

fn wrapped(text: &str, font: FontId, color: Color32, width: f32, rows: usize) -> LayoutJob {
    let mut job = LayoutJob::simple(text.to_owned(), font, color, width);
    job.wrap.max_rows = rows;
    job
}

/// Paints the card in `rect`. `opacity` fades it (the ghost that follows the pointer).
pub fn paint(painter: &Painter, rect: Rect, card: &LinkCard, opacity: f32) {
    let fade = |color: Color32| color.gamma_multiply(opacity);
    let accent = theme::text_primary();
    let words = CardText::of(card);
    painter.rect_filled(rect, CORNER, fade(theme::surface()));
    painter.rect_stroke(
        rect,
        CORNER,
        Stroke::new(1.0, fade(theme::border())),
        StrokeKind::Inside,
    );
    let painter = painter.with_clip_rect(rect.shrink(1.0));
    let inner = rect.shrink(PADDING);
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), HEADER_HEIGHT));
    icons::paint(
        &painter,
        Rect::from_min_size(header.min + vec2(0.0, 2.0), vec2(16.0, 16.0)),
        Icon::XLogo,
        fade(accent),
    );
    painter.text(
        header.min + vec2(22.0, HEADER_HEIGHT / 2.0),
        Align2::LEFT_CENTER,
        words.service,
        FontId::proportional(12.0),
        fade(theme::text_secondary()),
    );
    painter.text(
        header.right_center(),
        Align2::RIGHT_CENTER,
        &words.headline,
        FontId::proportional(12.0),
        fade(accent),
    );
    // The footer keeps its line; the title and the excerpt share what is above it.
    let footer_top = inner.bottom() - 14.0;
    let mut y = header.bottom() + 6.0;
    let title = painter.layout_job(wrapped(
        &words.title,
        FontId::proportional(13.0),
        fade(theme::text_primary()),
        inner.width(),
        2,
    ));
    let title_height = title.size().y;
    painter.galley(egui::pos2(inner.left(), y), title, Color32::WHITE);
    y += title_height + 4.0;
    if !words.excerpt.is_empty() && footer_top - y > 14.0 {
        let rows = (((footer_top - y - 4.0) / 15.0).floor() as usize).max(1);
        let excerpt = painter.layout_job(wrapped(
            &words.excerpt,
            FontId::proportional(12.0),
            fade(theme::text_secondary()),
            inner.width(),
            rows,
        ));
        painter.galley(egui::pos2(inner.left(), y), excerpt, Color32::WHITE);
    }
    if !words.footer.is_empty() {
        painter.text(
            egui::pos2(inner.left(), inner.bottom()),
            Align2::LEFT_BOTTOM,
            &words.footer,
            FontId::proportional(11.0),
            fade(theme::text_tertiary()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::link_card::{Link, LinkMeta};

    fn post() -> LinkCard {
        LinkCard::new(Link::parse("https://x.com/degenview/status/1750000000000000000").unwrap())
    }

    #[test]
    fn a_card_with_no_preview_says_only_what_its_link_proves() {
        let words = CardText::of(&post());
        assert_eq!(words.service, "X");
        assert_eq!(words.headline, "@degenview");
        assert_eq!(words.title, "Post 1750000000000000000");
        assert!(words.excerpt.is_empty(), "no text is made up");
        assert!(
            words.footer.starts_with("Preview not fetched · "),
            "{words:?}"
        );
        // A post numbered before snowflake ids has no time to show.
        let old = CardText::of(&LinkCard::new(
            Link::parse("https://x.com/jack/status/20").unwrap(),
        ));
        assert_eq!(old.footer, "Preview not fetched");
    }

    #[test]
    fn a_fetched_preview_replaces_the_link_derived_words_and_nothing_else() {
        let mut card = post();
        card.meta = Some(LinkMeta {
            title: "A title".into(),
            author: "Some Author".into(),
            excerpt: "The post's words".into(),
            fetched_at_ms: 1,
        });
        let words = CardText::of(&card);
        assert_eq!(words.title, "A title");
        assert_eq!(words.excerpt, "The post's words");
        assert!(words.footer.starts_with("Some Author · "), "{words:?}");
    }

    #[test]
    fn a_card_paints_inside_its_rectangle() {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        let bare = LinkCard::new(Link::parse("https://x.com/i/status/20").unwrap());
        for card in [post(), bare] {
            for size in [[180.0, 96.0], [300.0, 132.0], [640.0, 480.0]] {
                let rect = Rect::from_min_size(egui::pos2(20.0, 20.0), vec2(size[0], size[1]));
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(800.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| paint(&ui.painter_at(rect), rect, &card, 1.0),
                );
                output.textures_delta.clear();
                assert!(!output.shapes.is_empty());
                for clipped in &output.shapes {
                    assert!(
                        rect.expand(1.0).contains_rect(clipped.clip_rect),
                        "{size:?}"
                    );
                }
            }
        }
    }
}
