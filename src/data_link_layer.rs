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

    pub async fn send(&self, frame: Frame) {
        self.bus.con.reset();
        self.bus.bus_tx.send(frame).await;
        let _con = self.bus.con.wait().await;
    }

    pub async fn receive(&self) -> DataServiceInd {
        loop {
            let frame = self.bus.bus_rx.receive().await;
            info!("{}", frame);
            return DataServiceInd::Data(frame);
        }
    }
}
