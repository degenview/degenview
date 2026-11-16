//! The script library: the registry that lists scripts, names, the script picker, and the demo and
//! external scripts. The files themselves are `store::scripts`.

pub mod demo;
pub(in crate::pine_studio) mod disk_banner;
pub(in crate::pine_studio) mod empty_library;
pub(in crate::pine_studio) mod external;
pub mod name;
pub(in crate::pine_studio) mod picker;
pub mod prefs;
pub mod registry;
pub(in crate::pine_studio) mod unsaved;
