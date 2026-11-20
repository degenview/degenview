//! The script library: the script picker, and the demo and external scripts. The files, their
//! names and the libraries the compiler resolves are `store::scripts` and `model::pine::script_name`.

pub mod demo;
pub(in crate::pine_studio) mod disk_banner;
pub(in crate::pine_studio) mod empty_library;
pub(in crate::pine_studio) mod external;
pub(in crate::pine_studio) mod picker;
pub mod prefs;
pub(in crate::pine_studio) mod script_actions;
pub(in crate::pine_studio) mod unsaved;
