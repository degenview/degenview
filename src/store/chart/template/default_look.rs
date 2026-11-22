//! Which chart template new charts start from. It is one `setting` holding the template's id (a
//! preset's or a saved one's), so renaming or updating the template changes what new charts get.
//! No setting means a new chart starts from the app's own look.

use crate::model::ChartAppearance;
use crate::model::chart::template::{TemplateId, built_in};
use crate::store::Store;

/// Where the choice is kept (`sync::record::policy` lists it as a portable setting).
pub const KEY: &str = "chart.defaultTemplate";

/// The template new charts start from, as far as it can be resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DefaultTemplate {
    /// The template the user chose. It stays set when the template cannot be found, which is
    /// what a Mac sees while the template is still on its way from iCloud.
    pub id: Option<TemplateId>,
    /// Its look; `None` with no choice or when the choice no longer exists.
    pub look: Option<ChartAppearance>,
}

impl Store {
    /// The default template, resolved. A table this version cannot read counts as having none
    /// rather than failing a chart.
    pub fn default_chart_template(&self) -> DefaultTemplate {
        let Some(id) = self.setting::<TemplateId>(KEY) else {
            return DefaultTemplate::default();
        };
        let look = match built_in(id) {
            Some(preset) => Some(preset.appearance()),
            None => self
                .chart_templates_strict()
                .ok()
                .and_then(|saved| saved.into_iter().find(|template| template.id == id))
                .map(|template| template.appearance),
        };
        DefaultTemplate { id: Some(id), look }
    }

    /// Makes `id` the template new charts start from, or with `None` puts that back to the app's
    /// own look.
    pub fn set_default_chart_template(&self, id: Option<TemplateId>) -> rusqlite::Result<()> {
        match id {
            Some(id) => self.set_setting(KEY, &id),
            None => self.remove_setting(KEY),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart::canvas::BackgroundMode;
    use crate::model::chart::template::{BUILT_INS, ChartAppearanceTemplate};

    fn saved(id: TemplateId) -> ChartAppearanceTemplate {
        let mut appearance = ChartAppearance::default();
        appearance.canvas.background.mode = BackgroundMode::Gradient;
        ChartAppearanceTemplate {
            id,
            name: "Mine".into(),
            description: None,
            appearance,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn with_no_choice_a_new_chart_gets_the_app_look() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(store.default_chart_template(), DefaultTemplate::default());
    }

    #[test]
    fn a_saved_template_is_resolved_to_its_look_and_follows_updates() {
        let store = Store::open_in_memory().expect("open");
        let template = saved(2_000);
        store
            .replace_chart_templates(std::slice::from_ref(&template))
            .expect("saved");
        store.set_default_chart_template(Some(2_000)).expect("set");
        let chosen = store.default_chart_template();
        assert_eq!(chosen.id, Some(2_000));
        assert_eq!(chosen.look, Some(template.appearance));

        let mut updated = template;
        updated.appearance.canvas.margins.right = 33;
        store
            .replace_chart_templates(std::slice::from_ref(&updated))
            .expect("updated");
        assert_eq!(
            store.default_chart_template().look,
            Some(updated.appearance)
        );
    }

    #[test]
    fn a_preset_can_be_the_default() {
        let store = Store::open_in_memory().expect("open");
        let preset = &BUILT_INS[1];
        store
            .set_default_chart_template(Some(preset.id))
            .expect("set");
        assert_eq!(
            store.default_chart_template().look,
            Some(preset.appearance())
        );
    }

    #[test]
    fn a_template_that_is_gone_leaves_the_choice_but_no_look() {
        let store = Store::open_in_memory().expect("open");
        store.set_default_chart_template(Some(9_999)).expect("set");
        assert_eq!(
            store.default_chart_template(),
            DefaultTemplate {
                id: Some(9_999),
                look: None
            }
        );
    }

    #[test]
    fn clearing_the_choice_puts_the_app_look_back() {
        let store = Store::open_in_memory().expect("open");
        store
            .set_default_chart_template(Some(BUILT_INS[0].id))
            .expect("set");
        store.set_default_chart_template(None).expect("cleared");
        assert_eq!(store.default_chart_template(), DefaultTemplate::default());
    }
}
