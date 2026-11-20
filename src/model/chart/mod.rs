//! What one chart is made of: its look, canvas, viewport, price scale, layout, templates, snapshots and the bars it draws.

pub(in crate::model) mod appearance;
pub mod canvas;
pub mod clipboard;
pub mod context;
pub use crate::bars::kind;
pub(in crate::model) mod layout;
pub mod look;
pub mod object_tree;
pub(in crate::model) mod persisted;
pub mod price_scale;
pub(in crate::model) mod replay;
pub(in crate::model) mod saved_view;
pub mod snapshot;
pub mod sync;
pub use crate::bars::synthetic;
pub mod table;
pub mod template;
