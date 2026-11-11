//! Reusable drawing styles. A template is a named copy of one drawing's [`DrawingStyle`] for one
//! tool: its colours, thickness, fill, text style and the like, never its coordinates, text or
//! visibility, so applying it restyles a drawing and moves nothing. It follows TradingView's
//! drawing templates (save a drawing's look, apply it to another drawing of the same kind, make
//! it the tool's default).
//!
//! These are not chart appearance templates ([`crate::model::chart::template`]) and share only their
//! naming rules. Pure: no egui types, no storage.

use crate::model::chart::template::{TemplateError, TemplateId, clean_name};
use crate::model::drawings::style::DrawingStyle;
use crate::model::drawings::tool::Tool;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DrawingTemplate {
    pub id: TemplateId,
    pub name: String,
    pub tool: Tool,
    pub style: DrawingStyle,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// The user's saved drawing templates, in the order they were made, and every rule about
/// changing them. Names are unique per tool, ignoring case.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TemplateBook {
    templates: Vec<DrawingTemplate>,
}

impl TemplateBook {
    pub fn new(templates: Vec<DrawingTemplate>) -> Self {
        Self { templates }
    }

    pub fn saved(&self) -> &[DrawingTemplate] {
        &self.templates
    }

    /// The templates of `tool`, in the order they were made.
    pub fn for_tool(&self, tool: Tool) -> Vec<&DrawingTemplate> {
        self.templates
            .iter()
            .filter(|template| template.tool == tool)
            .collect()
    }

    pub fn get(&self, id: TemplateId) -> Option<&DrawingTemplate> {
        self.templates.iter().find(|template| template.id == id)
    }

    fn name_taken(&self, tool: Tool, name: &str, except: Option<TemplateId>) -> bool {
        let lowered = name.to_lowercase();
        self.templates.iter().any(|template| {
            template.tool == tool
                && Some(template.id) != except
                && template.name.to_lowercase() == lowered
        })
    }

    /// Saves `style` as a template of `tool` named `name`.
    pub fn save(
        &mut self,
        id: TemplateId,
        name: &str,
        tool: Tool,
        style: DrawingStyle,
        now: i64,
    ) -> Result<TemplateId, TemplateError> {
        let name = clean_name(name)?;
        if self.name_taken(tool, &name, None) || self.get(id).is_some() {
            return Err(TemplateError::DuplicateName);
        }
        self.templates.push(DrawingTemplate {
            id,
            name,
            tool,
            style: style.sanitized(),
            created_at: now,
            updated_at: now,
        });
        Ok(id)
    }

    pub fn rename(&mut self, id: TemplateId, name: &str, now: i64) -> Result<(), TemplateError> {
        let name = clean_name(name)?;
        let tool = self.get(id).ok_or(TemplateError::Missing)?.tool;
        if self.name_taken(tool, &name, Some(id)) {
            return Err(TemplateError::DuplicateName);
        }
        let template = self
            .templates
            .iter_mut()
            .find(|template| template.id == id)
            .ok_or(TemplateError::Missing)?;
        template.name = name;
        template.updated_at = now;
        Ok(())
    }

    pub fn delete(&mut self, id: TemplateId) -> Result<(), TemplateError> {
        let before = self.templates.len();
        self.templates.retain(|template| template.id != id);
        if self.templates.len() == before {
            return Err(TemplateError::Missing);
        }
        Ok(())
    }

    /// `base`, or `base 2`, `base 3`… the first free name for `tool`.
    pub fn free_name(&self, tool: Tool, base: &str) -> String {
        let base = base.trim();
        if !self.name_taken(tool, base, None) {
            return base.to_owned();
        }
        (2..)
            .map(|number| format!("{base} {number}"))
            .find(|name| !self.name_taken(tool, name, None))
            .unwrap_or_else(|| base.to_owned())
    }
}

/// The style a new drawing of `tool` starts with: the user's default for it, else the built-in.
pub fn style_for(
    tool: Tool,
    defaults: &std::collections::HashMap<Tool, DrawingStyle>,
) -> DrawingStyle {
    defaults
        .get(&tool)
        .copied()
        .unwrap_or_else(|| tool.default_style())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart::template::{FIRST_USER_ID, MAX_NAME_CHARS};

    fn style(width: f32) -> DrawingStyle {
        let mut style = Tool::Rectangle.default_style();
        style.stroke.width = width;
        style
    }

    #[test]
    fn templates_are_named_per_tool_and_unique_ignoring_case() {
        let mut book = TemplateBook::default();
        let a = FIRST_USER_ID;
        book.save(a, "  Support zone ", Tool::Rectangle, style(2.0), 5)
            .unwrap();
        assert_eq!(book.get(a).unwrap().name, "Support zone");
        assert_eq!(
            book.save(a + 1, "support ZONE", Tool::Rectangle, style(3.0), 6),
            Err(TemplateError::DuplicateName)
        );
        // The same name on another tool is fine.
        book.save(a + 1, "Support zone", Tool::Ellipse, style(3.0), 6)
            .unwrap();
        assert_eq!(book.for_tool(Tool::Rectangle).len(), 1);
        assert_eq!(book.for_tool(Tool::Ellipse).len(), 1);
        assert!(book.for_tool(Tool::Arc).is_empty());
        // An id in use is not reused.
        assert_eq!(
            book.save(a, "Other", Tool::Rectangle, style(1.0), 7),
            Err(TemplateError::DuplicateName)
        );
    }

    #[test]
    fn bad_names_are_refused() {
        let mut book = TemplateBook::default();
        assert_eq!(
            book.save(1, "  ", Tool::Arc, style(1.0), 0),
            Err(TemplateError::EmptyName)
        );
        assert_eq!(
            book.save(1, "a\nb", Tool::Arc, style(1.0), 0),
            Err(TemplateError::ControlCharacters)
        );
        assert_eq!(
            book.save(1, &"x".repeat(MAX_NAME_CHARS + 1), Tool::Arc, style(1.0), 0),
            Err(TemplateError::NameTooLong)
        );
        assert!(book.saved().is_empty());
    }

    #[test]
    fn rename_and_delete_follow_the_same_rules() {
        let mut book = TemplateBook::default();
        book.save(1, "One", Tool::Triangle, style(1.0), 0).unwrap();
        book.save(2, "Two", Tool::Triangle, style(2.0), 0).unwrap();
        assert_eq!(book.rename(2, "one", 9), Err(TemplateError::DuplicateName));
        assert_eq!(
            book.rename(2, "Two", 9),
            Ok(()),
            "keeping its own name is no clash"
        );
        book.rename(2, "Three", 9).unwrap();
        assert_eq!(book.get(2).unwrap().name, "Three");
        assert_eq!(book.get(2).unwrap().updated_at, 9);
        assert_eq!(book.rename(99, "Nope", 9), Err(TemplateError::Missing));
        book.delete(1).unwrap();
        assert_eq!(book.delete(1), Err(TemplateError::Missing));
        assert_eq!(book.saved().len(), 1);
    }

    #[test]
    fn a_template_holds_a_sanitized_style_and_no_coordinates() {
        let mut book = TemplateBook::default();
        let mut wild = style(2.0);
        wild.stroke.width = f32::NAN;
        book.save(1, "Wild", Tool::Rectangle, wild, 0).unwrap();
        assert_eq!(book.get(1).unwrap().style.stroke.width, 2.0);
        let json = serde_json::to_string(book.get(1).unwrap()).unwrap();
        assert!(
            !json.contains("time_ms") && !json.contains("\"price\""),
            "{json}"
        );
    }

    #[test]
    fn free_names_count_up() {
        let mut book = TemplateBook::default();
        assert_eq!(book.free_name(Tool::Arc, "My arc"), "My arc");
        book.save(1, "My arc", Tool::Arc, style(1.0), 0).unwrap();
        assert_eq!(book.free_name(Tool::Arc, "My arc"), "My arc 2");
        book.save(2, "My arc 2", Tool::Arc, style(1.0), 0).unwrap();
        assert_eq!(book.free_name(Tool::Arc, "my ARC"), "my ARC 3");
        assert_eq!(book.free_name(Tool::Curve, "My arc"), "My arc");
    }

    #[test]
    fn a_tools_style_is_its_default_unless_the_user_chose_one() {
        let mut defaults = std::collections::HashMap::new();
        assert_eq!(
            style_for(Tool::Rectangle, &defaults),
            Tool::Rectangle.default_style()
        );
        defaults.insert(Tool::Rectangle, style(7.0));
        assert_eq!(style_for(Tool::Rectangle, &defaults).stroke.width, 7.0);
        assert_eq!(style_for(Tool::Arc, &defaults), Tool::Arc.default_style());
    }
}
