//! KNX TP1 device stack on top of an onsemi NCN5130 transceiver.
#![cfg_attr(not(test), no_std)]

// Must come first: the logging macros are only visible in modules declared after it.
mod fmt;

pub mod application_layer;
pub mod bus;
pub mod data_link_layer;
pub mod data_point;
pub mod frame;
pub mod group_address_table;
pub mod group_object;
pub mod group_object_association_table;
pub mod ncn51_driver;
pub mod network_layer;
pub mod settings;
pub mod transport_layer;

#[cfg(test)]
mod test_util;
