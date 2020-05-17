//! UDP BitTorrent tracker protocol structures

#[cfg(test)]
extern crate quickcheck;
#[cfg(test)]
#[macro_use(quickcheck)]
extern crate quickcheck_macros;


pub mod converters;
pub mod types;