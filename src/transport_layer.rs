use crate::frame::*;
use crate::network_layer::{NetworkLayer, NetworkServiceInd, NetworkServiceReq};
use defmt::*;

pub struct TransportLayer {
    network: NetworkLayer,
}

/*pub enum TSAP {

}*/

#[derive(Format)]
pub enum TransportServiceInd {
    DataBroadcast(Frame),
    DataSystemBroadcast(Frame),
    DataGroup(Frame),
    DataTagGroup(Frame),
    DataIndividual(Frame),
    DataConnected(Frame, u8),
    /*Connect(Frame),
    Disconnect(Frame),
    ACK(Frame),
    NAK(Frame),*/
}

pub struct DataGroupReq {
    tsap: u8,
    frame: Frame,
}

impl DataGroupReq {
    pub fn new(tsap: u8, frame: Frame) -> Self {
        Self {
            tsap: tsap,
            frame: frame,
        }
    }
    fn info_frame(mut self, dst_address: GroupAddress) -> Frame {
        self.frame.set_dst_addr(&Address::Group(dst_address));
        self.frame.set_src_addr(&crate::settings::ADDRESS);
        self.frame.set_tpci(TpciBits::Six, 0x0);
        self.frame
    }
}

/*impl Info<network_layer::DataGroupReq> for DataGroupReq {

}*/

pub enum TransportServiceReq {
    DataGroupReq(DataGroupReq),
}

#[derive(Format)]
pub enum TransportLayerError {
    InvalidTpdu(u8),
}

impl TransportLayer {
    pub fn new(network: NetworkLayer) -> Self {
        Self { network: network }
    }

    pub async fn send(&self, req: TransportServiceReq) {
        match req {
            TransportServiceReq::DataGroupReq(req) => {
                // Hardcode this for now
                let dst_address = GroupAddress::from_parts(1, 1, 98);
                self.network
                    .send(NetworkServiceReq::DataGroup(req.info_frame(dst_address)))
                    .await;
            }
        }
    }

    pub async fn receive(&self) -> Result<TransportServiceInd, TransportLayerError> {
        loop {
            match self.network.receive().await {
                NetworkServiceInd::DataIndividual(frame) => {
                    let tpci = frame.tpci(TpciBits::Eight);
                    if tpci >> 2 == 0 {
                        return Ok(TransportServiceInd::DataIndividual(frame));
                    } else if tpci >> 6 == 1 {
                        let seq = (tpci >> 2) & 0xF;
                        return Ok(TransportServiceInd::DataConnected(frame, seq));
                    } else if tpci == 0x80 {
                        info!("T_Connect");
                    } else if tpci == 0x81 {
                        info!("T_Disconnect");
                    } else if tpci & 0x3c == 0xc2 {
                        info!("T_ACK");
                        let _seq = (tpci >> 2) & 0xF;
                    } else if tpci & 0x3c == 0xc4 {
                        info!("T_NACK");
                        let _seq = (tpci >> 2) & 0xF;
                    } else {
                        return Err(TransportLayerError::InvalidTpdu(tpci));
                    }
                }
                NetworkServiceInd::DataBroadcast(frame) => {
                    let tpci = frame.tpci(TpciBits::Six);
                    if tpci == 0 {
                        return Ok(TransportServiceInd::DataBroadcast(frame));
                    } else {
                        return Err(TransportLayerError::InvalidTpdu(tpci));
                    }
                }
                NetworkServiceInd::DataGroup(frame) => {
                    let tpci = frame.tpci(TpciBits::Six);
                    info!("DataGroup: {:x}", tpci);
                    match tpci {
                        0 => return Ok(TransportServiceInd::DataGroup(frame)),
                        1 => return Ok(TransportServiceInd::DataTagGroup(frame)),
                        _ => return Err(TransportLayerError::InvalidTpdu(tpci)),
                    }
                }
                NetworkServiceInd::DataSystemBroadcast(frame) => {
                    return Ok(TransportServiceInd::DataSystemBroadcast(frame))
                }
            }
        }
    }
}
