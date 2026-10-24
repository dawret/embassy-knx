//! Helpers for unit tests: frame pool setup, a time-limited executor and a harness that plays
//! the driver's side of the [`Bus`].
use crate::bus::{Bus, ConStatus};
use crate::data_link_layer::DataLinkLayer;
use crate::frame::{init_frame_pool, Frame, FrameBlock};
use crate::network_layer::NetworkLayer;
use crate::transport_layer::TransportLayer;
use core::future::Future;
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer, WithTimeout};
use std::sync::Once;
use std::vec::Vec;

/// Shared by all tests in the process, which run in parallel.
const TEST_POOL_SIZE: usize = 64;

pub fn init_pool() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let blocks = Box::leak(Box::new([const { FrameBlock::new() }; TEST_POOL_SIZE]));
        init_frame_pool(blocks);
    });
}

/// Runs `fut` to completion and fails the test if that takes longer than two seconds.
pub fn run<F: Future>(fut: F) -> F::Output {
    futures_executor::block_on(fut.with_timeout(Duration::from_secs(2))).expect("test timed out")
}

/// Appends the KNX checksum (inverted XOR of all octets) to a telegram.
pub fn with_checksum(telegram: &[u8]) -> Vec<u8> {
    let mut bytes = telegram.to_vec();
    bytes.push(!telegram.iter().fold(0, |acc, b| acc ^ b));
    bytes
}

/// Builds a received frame from a telegram given without its checksum.
pub fn frame(telegram: &[u8]) -> Frame {
    init_pool();
    Frame::from_slice(&with_checksum(telegram)).unwrap()
}

/// Real data link, network and transport layers on a private [`Bus`]. The harness stands in
/// for the driver: it injects received frames and confirms every frame the stack sends.
pub struct StackHarness {
    pub bus: &'static Bus,
    sent: Channel<CriticalSectionRawMutex, Vec<u8>, 16>,
}

impl StackHarness {
    pub fn new() -> (&'static Self, TransportLayer) {
        init_pool();
        let harness: &'static Self = Box::leak(Box::new(Self {
            bus: Box::leak(Box::new(Bus::new())),
            sent: Channel::new(),
        }));
        let network = NetworkLayer::new(DataLinkLayer::new(harness.bus));
        (harness, TransportLayer::new(network))
    }

    /// Delivers a telegram (given without checksum) as if the driver had received it.
    pub fn inject(&self, telegram: &[u8]) {
        self.bus.bus_rx.try_send(frame(telegram)).unwrap();
    }

    /// Next frame the stack sent, without the checksum octet.
    pub async fn sent(&self) -> Vec<u8> {
        self.sent.receive().await
    }

    pub fn nothing_sent(&self) -> bool {
        self.sent.is_empty()
    }

    /// Runs `body` while the harness answers the stack's sends with L_Data.con(Ok).
    pub async fn run<F: Future>(&self, body: F) -> F::Output {
        let driver = async {
            loop {
                let frame = self.bus.bus_tx.receive().await;
                let data = frame.data();
                self.sent.send(data[..data.len() - 1].to_vec()).await;
                self.bus.con.signal(ConStatus::Ok);
            }
        };
        match select(body, driver).await {
            Either::First(out) => out,
            Either::Second(_) => unreachable!(),
        }
    }
}

/// Waits briefly, so that frames the stack might still send become visible.
pub async fn settle() {
    Timer::after(Duration::from_millis(20)).await;
}
