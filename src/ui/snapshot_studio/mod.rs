//! The Screenshot Studio: the image it makes of a chart or a layout ([`compose`]), from a capture
//! frozen at the moment it was taken ([`capture`]).

pub mod capture;
pub mod compose;
pub mod previews;
pub mod window;

#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod cost_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod gallery_tests;
