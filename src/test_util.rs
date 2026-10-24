use crate::bus::{Bus, ConStatus};
use crate::data_link_layer::DataLinkLayer;
use crate::frame::{
    init_frame_pool, Frame, FrameError, GroupAddress, LargeFrameBlock, SmallFrameBlock,
};
use crate::group_address_table::GroupAddressTable;
use crate::group_object::{value_bytes, Descriptor, GroupObjectRam, GroupObjects};
use crate::network_layer::NetworkLayer;
use crate::transport_layer::{at, TransportLayer, TransportServiceInd};
use core::future::Future;
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer, WithTimeout};
use std::sync::Once;
use std::vec::Vec;

const TEST_POOL_SIZE: usize = 64;

/// 1/1/98.
pub const GROUP_ADDRESSES: GroupAddressTable =
    GroupAddressTable::new(&[GroupAddress::from_parts(1, 1, 98)]);

pub fn init_pool() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let small = Box::leak(Box::new([const { SmallFrameBlock::new() }; TEST_POOL_SIZE]));
        let large = Box::leak(Box::new([const { LargeFrameBlock::new() }; TEST_POOL_SIZE]));
        init_frame_pool(small, large);
    });
}

pub fn run<F: Future>(fut: F) -> F::Output {
    futures_executor::block_on(fut.with_timeout(Duration::from_secs(2))).expect("test timed out")
}

pub fn with_checksum(telegram: &[u8]) -> Vec<u8> {
    let mut bytes = telegram.to_vec();
    bytes.push(!telegram.iter().fold(0, |acc, b| acc ^ b));
    bytes
}

pub fn frame(telegram: &[u8]) -> Frame {
    init_pool();
    Frame::from_slice(&with_checksum(telegram)).unwrap()
}

pub fn group_objects(descriptors: &[Descriptor]) -> &'static GroupObjects {
    let descriptors: &'static [Descriptor] = Box::leak(descriptors.to_vec().into_boxed_slice());
    let values = Box::leak(vec![0; value_bytes(descriptors)].into_boxed_slice());
    let ram = Box::leak(vec![GroupObjectRam::new(); descriptors.len()].into_boxed_slice());
    Box::leak(Box::new(GroupObjects::new(descriptors, values, ram)))
}

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
        (harness, TransportLayer::new(network, GROUP_ADDRESSES))
    }

    pub fn inject(&self, telegram: &[u8]) {
        self.bus.bus_rx.try_send(frame(telegram)).unwrap();
    }

    pub async fn sent(&self) -> Vec<u8> {
        self.sent.receive().await
    }

    pub fn nothing_sent(&self) -> bool {
        self.sent.is_empty()
    }

    pub async fn run<F: Future>(&self, body: F) -> F::Output {
        let driver = async {
            loop {
                let frame = self.bus.bus_tx.receive().await;
                let data = frame.data();
                self.sent.send(data[..data.len() - 1].to_vec()).await;
                self.bus.con.signal((frame, ConStatus::Ok));
            }
        };
        match select(body, driver).await {
            Either::First(out) => out,
            Either::Second(_) => unreachable!(),
        }
    }
}

pub async fn receive(tl: &TransportLayer) -> Result<TransportServiceInd, FrameError> {
    loop {
        let ind = match select(tl.next_frame(), at(tl.deadline())).await {
            Either::First(frame) => tl.handle_frame(frame).await,
            Either::Second(()) => tl.handle_timeout().await,
        };
        if let Some(ind) = ind? {
            return Ok(ind);
        }
    }
}

pub async fn settle() {
    Timer::after(Duration::from_millis(20)).await;
}
