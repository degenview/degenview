//! The trading screens every venue shares: the order sheet and the pieces it is made of. They are
//! drawn from `crate::trading` only, so a venue added there appears here with no change.

pub mod connect_list;
pub mod fields;
pub mod order_sheet;
pub mod venue_pill;

#[cfg(test)]
mod sheet_gallery;
