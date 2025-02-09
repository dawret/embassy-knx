use crate::data_link_layer::{DataLinkLayer, DataServiceInd};
use crate::frame::*;
use defmt::*;

pub struct NetworkLayer {
    data_link: DataLinkLayer,
}

pub enum NetworkServiceReq {
    DataGroup(Frame),
}

#[derive(Format)]
pub enum NetworkServiceInd {
    DataIndividual(Frame),
    DataGroup(Frame),
    DataBroadcast(Frame),
    DataSystemBroadcast(Frame),
}

impl NetworkLayer {
    pub fn new(data_link: DataLinkLayer) -> Self {
        Self {
            data_link: data_link,
        }
    }

    pub async fn receive(&self) -> NetworkServiceInd {
        loop {
            match self.data_link.receive().await {
                DataServiceInd::Data(frame) => match frame.dst_addr() {
                    Address::Individual(_) => return NetworkServiceInd::DataIndividual(frame),
                    Address::Group(ref addr) => {
                        if addr == &GroupAddress::new(0) {
                            return NetworkServiceInd::DataBroadcast(frame);
                        } else {
                            return NetworkServiceInd::DataGroup(frame);
                        }
                    }
                    _ => {}
                },
                DataServiceInd::SystemBroadcast(frame) => {
                    return NetworkServiceInd::DataSystemBroadcast(frame)
                }
                _ => {}
            }
        }
    }

    pub async fn send(&self, req: NetworkServiceReq) {
        match req {
            NetworkServiceReq::DataGroup(frame) => self.data_link.send(frame).await,
        }
    }
}
