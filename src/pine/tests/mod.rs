//! End-to-end tests of the engine that belong to no single module.

#[cfg(test)]
pub(in crate::pine) mod bench;
#[cfg(test)]
pub(in crate::pine) mod chart_type;
#[cfg(feature = "corpus")]
pub(in crate::pine) mod corpus;
#[cfg(test)]
pub(in crate::pine) mod language;
#[cfg(test)]
pub(in crate::pine) mod varip;
