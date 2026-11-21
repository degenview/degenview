//! Response shapes that more than one venue speaks. A venue whose API is another's clone (Aster's
//! futures API is Binance's) configures the shape in its own directory instead of copying its
//! parsers; what differs between the two stays with each venue.

pub(in crate::service) mod binance;
