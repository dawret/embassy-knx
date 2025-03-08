use crate::frame::*;
use crate::ncn51_driver::{ConStatus, CON_SIGNAL};
use defmt::*;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::{Receiver, Sender};

pub struct DataLinkLayer {
    rx: Receiver<'static, ThreadModeRawMutex, Frame, 8>,
    tx: Sender<'static, ThreadModeRawMutex, Frame, 8>,
}

#[derive(Format)]
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
    pub fn new(
        rx: Receiver<'static, ThreadModeRawMutex, Frame, 8>,
        tx: Sender<'static, ThreadModeRawMutex, Frame, 8>,
    ) -> Self {
        Self { rx: rx, tx: tx }
    }

    pub async fn send(&self, frame: Frame) {
        CON_SIGNAL.reset();
        self.tx.send(frame).await;
        let con = CON_SIGNAL.wait().await;
    }

    pub async fn receive(&self) -> DataServiceInd {
        loop {
            let frame = self.rx.receive().await;
            info!("{}", frame);
            return DataServiceInd::Data(frame);
        }
    }
}
