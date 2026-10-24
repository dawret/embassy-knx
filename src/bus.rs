use crate::frame::Frame;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;

pub const BUS_CHANNEL_SIZE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ConStatus {
    Ok,
    NotOk,
}

/// Queues between the NCN5130 driver and the stack, named from the bus's point of view.
pub struct Bus {
    pub bus_rx: Channel<CriticalSectionRawMutex, Frame, BUS_CHANNEL_SIZE>,
    pub bus_tx: Channel<CriticalSectionRawMutex, Frame, BUS_CHANNEL_SIZE>,
    /// L_Data.con for the last frame taken from `bus_tx`.
    pub con: Signal<CriticalSectionRawMutex, ConStatus>,
}

impl Bus {
    pub const fn new() -> Self {
        Self {
            bus_rx: Channel::new(),
            bus_tx: Channel::new(),
            con: Signal::new(),
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}
