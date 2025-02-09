use defmt::*;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::{Channel, Sender};
use embassy_sync::lazy_lock::LazyLock;
use embassy_sync::signal::Signal;

#[derive(PartialEq, Format)]
pub enum GroupObjectState {
    ReadRequest,
    WriteRequest,
    Transmitting,
    Ok,
    Update,
    Error,
}

pub struct GroupObject {
    state: GroupObjectState,
    signal: Signal<ThreadModeRawMutex, GroupObjectState>,
    value: u8,
    sender: LazyLock<
        Sender<
            'static,
            ThreadModeRawMutex,
            &'static Signal<ThreadModeRawMutex, GroupObjectState>,
            4,
        >,
    >,
}

impl GroupObject {
    pub async fn read_value(&'static mut self) {
        // TODO: Check initial state
        self.state = GroupObjectState::Transmitting;
        self.sender.get().send(&self.signal).await;
        self.state = self.signal.wait().await;
        if self.state != GroupObjectState::Ok {
            info!("Invalid state: {:?}", self.state);
        }
    }
}

pub static GROUP_OBJECT_CHANNEL: Channel<
    ThreadModeRawMutex,
    &'static Signal<ThreadModeRawMutex, GroupObjectState>,
    4,
> = Channel::new();

pub static GROUP_OBJECTS: [GroupObject; 2] = [
    GroupObject {
        state: GroupObjectState::Ok,
        signal: Signal::new(),
        value: 0,
        sender: LazyLock::new(|| GROUP_OBJECT_CHANNEL.sender()),
    },
    GroupObject {
        state: GroupObjectState::Ok,
        signal: Signal::new(),
        value: 1,
        sender: LazyLock::new(|| GROUP_OBJECT_CHANNEL.sender()),
    },
];
