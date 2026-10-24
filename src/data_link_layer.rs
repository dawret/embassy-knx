use crate::bus::{Bus, ConStatus};
use crate::frame::*;

pub struct DataLinkLayer {
    bus: &'static Bus,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DataServiceInd {
    Data(Frame),
    SystemBroadcast(Frame),
    Busmon(Frame),
    ServiceInformation(Frame),
}

pub enum DataServiceCon {
    Data(ConStatus),
}

impl DataLinkLayer {
    pub fn new(bus: &'static Bus) -> Self {
        Self { bus }
    }

    pub async fn send(&self, frame: Frame) -> (Frame, ConStatus) {
        self.bus.con.reset();
        self.bus.bus_tx.send(frame).await;
        self.bus.con.wait().await
    }

    /// Cancel-safe: only takes the next frame off the queue.
    pub async fn next_frame(&self) -> Frame {
        self.bus.bus_rx.receive().await
    }

    pub fn receive(&self, frame: Frame) -> DataServiceInd {
        DataServiceInd::Data(frame)
    }
}
