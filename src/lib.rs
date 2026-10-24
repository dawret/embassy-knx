//! KNX TP1 device stack on top of an onsemi NCN5130 transceiver.
//!
//! The crate is `no_std` and HAL-free: the driver talks to the transceiver through
//! `embedded_io_async::{Read, Write}`, so the whole stack also runs on the host
//! for testing (see `docs/testing.md`).
#![cfg_attr(not(test), no_std)]

// Must come first: the logging macros are only visible in modules declared after it.
mod fmt;

pub mod application_layer;
pub mod bus;
pub mod data_link_layer;
pub mod data_point;
pub mod frame;
pub mod ncn51_driver;
pub mod network_layer;
pub mod settings;
pub mod transport_layer;

#[cfg(test)]
mod test_util;
