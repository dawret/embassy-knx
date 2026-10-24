use crate::bus::ConStatus;
use crate::data_link_layer::{DataLinkLayer, DataServiceInd};
use crate::frame::*;

pub struct NetworkLayer {
    data_link: DataLinkLayer,
}

pub enum NetworkServiceReq {
    DataGroup(Frame),
    DataIndividual(Frame),
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum NetworkServiceInd {
    DataIndividual(Frame),
    DataGroup(Frame),
    DataBroadcast(Frame),
    DataSystemBroadcast(Frame),
}

impl NetworkLayer {
    pub fn new(data_link: DataLinkLayer) -> Self {
        Self { data_link }
    }

    pub async fn next_frame(&self) -> Frame {
        self.data_link.next_frame().await
    }

    pub fn receive(&self, frame: Frame) -> Option<NetworkServiceInd> {
        match self.data_link.receive(frame)? {
            DataServiceInd::Data(frame) => match frame.dst_addr() {
                Address::Individual(_) => Some(NetworkServiceInd::DataIndividual(frame)),
                Address::Group(addr) if addr == GroupAddress::new(0) => {
                    Some(NetworkServiceInd::DataBroadcast(frame))
                }
                Address::Group(_) => Some(NetworkServiceInd::DataGroup(frame)),
            },
            DataServiceInd::SystemBroadcast(frame) => {
                Some(NetworkServiceInd::DataSystemBroadcast(frame))
            }
            _ => None,
        }
    }

    pub async fn send(&self, req: NetworkServiceReq) -> (Frame, ConStatus) {
        match req {
            NetworkServiceReq::DataGroup(frame) => self.data_link.send(frame).await,
            NetworkServiceReq::DataIndividual(frame) => self.data_link.send(frame).await,
        }
    }
}
