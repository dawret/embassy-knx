use crate::data_point::*;
use crate::transport_layer::{TransportLayer, TransportServiceInd};
use crate::{frame::*, transport_layer};
use defmt::*;
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::{Receiver, Sender};

pub enum ApplicationServiceInd {
    GroupValueRead(u8 /*ASAP */),
}

pub struct GroupReadResponse {
    data: DataPoint,
    asap: u8,
    hop_count: u8,
    priority: Priority,
}

impl GroupReadResponse {
    pub fn new(asap: u8, data: DataPoint, priority: Priority) -> Self {
        Self {
            asap: asap,
            data: data,
            hop_count: 7,
            priority: priority,
        }
    }

    fn to_transport(self, tsap: u8) -> Result<transport_layer::DataGroupReq, FrameError> {
        let mut frame = Frame::from_datapoint(&self.data)?;
        frame.set_apci(ApciBits::Four, 0x1);
        frame.set_priority(self.priority);
        frame.set_hop_count(self.hop_count);
        Ok(transport_layer::DataGroupReq::new(tsap, frame))
    }
}

pub enum ApplicationServiceRes {
    GroupValueRead(GroupReadResponse),
}

pub struct ApplicationLayer {
    transport: TransportLayer,
    rx: Receiver<'static, ThreadModeRawMutex, ApplicationServiceRes, 4>,
    tx: Sender<'static, ThreadModeRawMutex, ApplicationServiceInd, 4>,
}

impl ApplicationLayer {
    pub fn new(
        transport: TransportLayer,
        rx: Receiver<'static, ThreadModeRawMutex, ApplicationServiceRes, 4>,
        tx: Sender<'static, ThreadModeRawMutex, ApplicationServiceInd, 4>,
    ) -> Self {
        Self {
            transport: transport,
            rx: rx,
            tx: tx,
        }
    }

    pub async fn receive(
        &self,
        frame: Result<TransportServiceInd, transport_layer::TransportLayerError>,
    ) {
        match frame {
            Ok(TransportServiceInd::DataGroup(frame)) => {
                let apci = frame.apci(ApciBits::Four);
                match apci {
                    0 => {
                        info!("A_GroupValue_Read");
                        self.tx.send(ApplicationServiceInd::GroupValueRead(0)).await;
                    }
                    1 => {
                        info!("A_GroupValue_Response");
                    }
                    2 => {
                        info!("A_GroupValue_Write");
                    }
                    _ => {
                        error!("Invalid APCI: {:x}", apci);
                    }
                }
            }
            Ok(_) => {}
            Err(e) => error!("Frame reception error: {}", e),
        }
    }

    pub async fn run(self) -> ! {
        loop {
            match select(self.transport.receive(), self.rx.ready_to_receive()).await {
                Either::First(frame) => {
                    self.receive(frame).await;
                }
                Either::Second(_) => match self.rx.receive().await {
                    ApplicationServiceRes::GroupValueRead(resp) => {
                        self.transport
                            .send(transport_layer::TransportServiceReq::DataGroupReq(unwrap!(
                                resp.to_transport(0),
                            )))
                            .await;
                    }
                },
            }
        }
    }
}
