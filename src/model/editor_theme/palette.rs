use super::{Rgba, SurfaceColor, SyntaxRole};

/// One editable colour of a theme: a syntax role or a surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ColorKey {
    Syntax(SyntaxRole),
    Surface(SurfaceColor),
}

impl ColorKey {
    /// The name a theme file and a saved override use: the role's or the surface's.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Syntax(role) => role.key(),
            Self::Surface(surface) => surface.key(),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Syntax(role) => role.label(),
            Self::Surface(surface) => surface.label(),
        }
    }

    pub const fn hint(self) -> &'static str {
        match self {
            Self::Syntax(role) => role.hint(),
            Self::Surface(surface) => surface.hint(),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        SyntaxRole::from_key(key)
            .map(Self::Syntax)
            .or_else(|| SurfaceColor::from_key(key).map(Self::Surface))
    }
}

/// Every colour of a theme.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Palette {
    pub syntax: [Rgba; SyntaxRole::COUNT],
    pub surfaces: [Rgba; SurfaceColor::COUNT],
}

impl Palette {
    pub fn get(&self, key: ColorKey) -> Rgba {
        match key {
            ColorKey::Syntax(role) => self.syntax[role.index()],
            ColorKey::Surface(surface) => self.surfaces[surface.index()],
        }
    }

    pub fn set(&mut self, key: ColorKey, color: Rgba) {
        match key {
            ColorKey::Syntax(role) => self.syntax[role.index()] = color,
            ColorKey::Surface(surface) => self.surfaces[surface.index()] = color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_find_their_colours() {
        for role in SyntaxRole::ALL {
            assert_eq!(ColorKey::from_key(role.key()), Some(ColorKey::Syntax(role)));
        }
        for surface in SurfaceColor::ALL {
            assert_eq!(
                ColorKey::from_key(surface.key()),
                Some(ColorKey::Surface(surface))
            );
        }
        assert_eq!(ColorKey::from_key("sparkle"), None);
        assert_eq!(ColorKey::Syntax(SyntaxRole::Keyword).label(), "Keywords");
        assert!(
            ColorKey::Surface(SurfaceColor::Caret)
                .hint()
                .contains("cursor")
        );
    }

    #[test]
    fn a_palette_reads_and_writes_by_key() {
        let mut palette = crate::model::editor_theme::BuiltInTheme::Dark.palette();
        let key = ColorKey::Syntax(SyntaxRole::Keyword);
        palette.set(key, Rgba::hex(0x123456));
        assert_eq!(palette.get(key), Rgba::hex(0x123456));
        assert_ne!(
            palette.get(ColorKey::Syntax(SyntaxRole::ControlKeyword)),
            Rgba::hex(0x123456)
        );
    }
}
