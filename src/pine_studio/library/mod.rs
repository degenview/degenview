//! The script library: where scripts live on disk, the registry that lists them, the sidebar, and the demo and external scripts.

pub mod demo;
pub(in crate::pine_studio) mod disk_banner;
pub(in crate::pine_studio) mod empty_library;
pub(in crate::pine_studio) mod external;
pub mod name;
pub mod prefs;
pub mod registry;
pub(in crate::pine_studio) mod sidebar;
pub mod store;
pub(in crate::pine_studio) mod unsaved;
