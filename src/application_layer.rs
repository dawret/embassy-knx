use crate::bus::ConStatus;
use crate::frame::*;
use crate::group_address_table::Tsap;
use crate::group_object::{Asap, GroupObjects};
use crate::group_object_association_table::AssociationTable;
use crate::transport_layer::{at, TransportLayer, TransportServiceInd, TransportServiceReq};
use embassy_futures::select::{select3, Either3};

pub struct ApplicationLayer {
    transport: TransportLayer,
    associations: AssociationTable,
    group_objects: &'static GroupObjects,
}

impl ApplicationLayer {
    pub fn new(
        transport: TransportLayer,
        associations: AssociationTable,
        group_objects: &'static GroupObjects,
    ) -> Self {
        Self {
            transport,
            associations,
            group_objects,
        }
    }

    async fn handle_ind(&self, ind: Result<Option<TransportServiceInd>, FrameError>) {
        match ind {
            Ok(Some(TransportServiceInd::DataGroup { tsap, frame })) => {
                self.group_value_ind(tsap, frame).await
            }
            Ok(_) => {}
            Err(e) => error!("Frame reception error: {}", e),
        }
    }

    async fn group_value_ind(&self, tsap: Tsap, frame: Frame) {
        match frame.group_service() {
            Some(GroupService::Read) => {
                drop(frame);
                for asap in self.associations.asaps(tsap) {
                    if let Some(response) = self.group_objects.response_frame(asap) {
                        self.send(asap, response).await;
                    }
                }
            }
            Some(service) => {
                for asap in self.associations.asaps(tsap) {
                    self.group_objects.bus_value(asap, service, &frame);
                }
            }
            None => warn!("Unsupported group APCI: {:x}", frame.apci(ApciBits::Ten)),
        }
    }

    async fn send(&self, asap: Asap, frame: Frame) -> Option<(Tsap, Frame, ConStatus)> {
        let Some(tsap) = self.associations.sending_tsap(asap) else {
            warn!("Group object {} has no group address", asap);
            return None;
        };
        let (frame, status) = self
            .transport
            .send(TransportServiceReq::DataGroup { tsap, frame })
            .await?;
        Some((tsap, frame, status))
    }

    /// Sends one transmit request; the others wait for the next loop iteration, so received
    /// frames are handled in between.
    async fn transmit(&self) {
        let Some((asap, frame)) = self.group_objects.start_transmission() else {
            return;
        };
        let write = frame.group_service() == Some(GroupService::Write);
        match self.send(asap, frame).await {
            Some((tsap, frame, status)) => {
                self.group_objects.confirm(asap, status);
                if write && status == ConStatus::Ok {
                    for other in self.associations.asaps(tsap).filter(|&a| a != asap) {
                        self.group_objects.bus_value(other, GroupService::Write, &frame);
                    }
                }
            }
            None => self.group_objects.confirm(asap, ConStatus::NotOk),
        }
        if self.group_objects.transmit_pending() {
            self.group_objects.signal_transmit_request();
        }
    }

    /// Each `select` branch only takes an event off a queue; all processing happens after it.
    pub async fn run(self) -> ! {
        self.group_objects.request_reads_on_init();
        loop {
            let event = select3(
                self.transport.next_frame(),
                self.group_objects.transmit_requested(),
                at(self.transport.deadline()),
            )
            .await;
            match event {
                Either3::First(frame) => {
                    let ind = self.transport.handle_frame(frame).await;
                    self.handle_ind(ind).await;
                }
                Either3::Second(()) => self.transmit().await,
                Either3::Third(()) => {
                    let ind = self.transport.handle_timeout().await;
                    self.handle_ind(ind).await;
                }
            }
        }
    }
}
