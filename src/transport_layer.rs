use crate::frame::*;
use crate::network_layer::{NetworkLayer, NetworkServiceInd, NetworkServiceReq};
use core::cell::RefCell;
use defmt::*;
use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Ticker};
use futures::future;

pub struct TransportLayer {
    network: NetworkLayer,
    connection: RefCell<Connection>,
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
    DataConnected(Frame),
    Connect(Frame),
    Disconnect(Frame),
    ACK(Frame),
    NAK(Frame),
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

impl TransportLayer {
    pub fn new(network: NetworkLayer) -> Self {
        Self {
            network,
            connection: RefCell::new(Connection::new()),
        }
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
    pub async fn receive_ind(
        &self,
        ind: NetworkServiceInd,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        match ind {
            NetworkServiceInd::DataIndividual(frame) => {
                let tpci = frame.tpci(TpciBits::Eight);
                if tpci >> 2 == 0 {
                    Ok(Some(TransportServiceInd::DataIndividual(frame)))
                } else if tpci >> 6 == 1 {
                    info!("T_DataConnected");
                    self.connection
                        .borrow_mut()
                        .handle_ind(TransportServiceInd::DataConnected(frame), &self.network)
                        .await
                } else if tpci == 0x80 {
                    info!("T_Connect");
                    self.connection
                        .borrow_mut()
                        .handle_ind(TransportServiceInd::Connect(frame), &self.network)
                        .await
                } else if tpci == 0x81 {
                    info!("T_Disconnect");
                    self.connection
                        .borrow_mut()
                        .handle_ind(TransportServiceInd::Disconnect(frame), &self.network)
                        .await
                } else if tpci & 0x3c == 0xc2 {
                    info!("T_ACK");
                    self.connection
                        .borrow_mut()
                        .handle_ind(TransportServiceInd::ACK(frame), &self.network)
                        .await
                } else if tpci & 0x3c == 0xc4 {
                    info!("T_NACK");
                    self.connection
                        .borrow_mut()
                        .handle_ind(TransportServiceInd::NAK(frame), &self.network)
                        .await
                    //let _seq = (tpci >> 2) & 0xF;
                } else {
                    Err(FrameError::InvalidTpdu(tpci))
                }
            }
            NetworkServiceInd::DataBroadcast(frame) => {
                let tpci = frame.tpci(TpciBits::Six);
                if tpci == 0 {
                    Ok(Some(TransportServiceInd::DataBroadcast(frame)))
                } else {
                    Err(FrameError::InvalidTpdu(tpci))
                }
            }
            NetworkServiceInd::DataGroup(frame) => {
                let tpci = frame.tpci(TpciBits::Six);
                info!("DataGroup: {:x}", tpci);
                match tpci {
                    0 => Ok(Some(TransportServiceInd::DataGroup(frame))),
                    1 => Ok(Some(TransportServiceInd::DataTagGroup(frame))),
                    _ => Err(FrameError::InvalidTpdu(tpci)),
                }
            }
            NetworkServiceInd::DataSystemBroadcast(frame) => {
                Ok(Some(TransportServiceInd::DataSystemBroadcast(frame)))
            }
        }
    }

    pub async fn receive(&self) -> Result<TransportServiceInd, FrameError> {
        loop {
            let ret = match select(
                self.network.receive(),
                self.connection
                    .borrow_mut()
                    .connection_timeout
                    .as_mut()
                    .map_or(future::Either::Left(core::future::pending()), |f| {
                        future::Either::Right(f.next())
                    }),
            )
            .await
            {
                Either::First(ind) => self.receive_ind(ind).await,
                Either::Second(_) => self.connection.borrow_mut().connection_timeout().await,
            }?;
            if let Some(ind) = ret {
                return Ok(ind);
            }
        }
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
enum Events {
    ConnectReqSameAddress_E00,
    ConnectReqNewAddress_E01,
    DisconnectReqSameAddress_E02,
    DisconnectReqNewAddress_E03,
    DataConnected_E04,
    DataConnected_E05,
    DataConnected_E06,
    DataConnectedNewAddress_E07,
    Ack_E08,
    Ack_E09,
    AckNewAddress_E10,
    Nak_E11,
    Nak_E12,
    Nak_E13,
    NakNewAddress_E14,
    DataConnected_E15,
    E16,
    E17,
    E18,
    E19,
    E20,
    E21,
    E22,
    E23,
    E24,
    E25,
    E26,
    E27,
}

enum States {
    Closed,
    OpenIdle,
    OpenWait,
    Connecting,
}

struct Connection {
    state: States,
    seq_no_send: u8,
    seq_no_recv: u8,
    rep_count: u8,
    src_addr: Option<IndividualAddress>,
    connection_timeout: Option<Ticker>,
    stored_frame: Option<Frame>,
}

impl Connection {
    const MAX_REP_COUNT: u8 = 3;
    const CONNECTION_TIMEOUT_SEC: u64 = 6;
    fn new() -> Self {
        Connection {
            state: States::Closed,
            seq_no_send: 0,
            seq_no_recv: 0,
            rep_count: 0,
            src_addr: None,
            connection_timeout: None,
            stored_frame: None,
        }
    }

    async fn handle_ind(
        &mut self,
        ind: TransportServiceInd,
        network: &NetworkLayer,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        let (event, frame) = match ind {
            TransportServiceInd::Connect(frame) => match &self.src_addr {
                Some(addr) if addr == &frame.src_addr() => {
                    (Events::ConnectReqSameAddress_E00, frame)
                }
                _ => (Events::ConnectReqNewAddress_E01, frame),
            },
            TransportServiceInd::Disconnect(frame) => match &self.src_addr {
                Some(addr) if addr == &frame.src_addr() => {
                    (Events::DisconnectReqSameAddress_E02, frame)
                }
                _ => (Events::DisconnectReqNewAddress_E03, frame),
            },
            TransportServiceInd::DataConnected(frame) => match &self.src_addr {
                Some(addr) if addr == &frame.src_addr() => match frame.tpci_seq() {
                    val if val == self.seq_no_recv => (Events::DataConnected_E04, frame),
                    val if val == self.seq_no_recv - 1 => (Events::DataConnected_E05, frame),
                    _ => (Events::DataConnected_E06, frame),
                },
                _ => (Events::DataConnectedNewAddress_E07, frame),
            },
            TransportServiceInd::ACK(frame) => match &self.src_addr {
                Some(addr) if addr == &frame.src_addr() => {
                    if frame.tpci_seq() == self.seq_no_send {
                        (Events::Ack_E08, frame)
                    } else {
                        (Events::Ack_E09, frame)
                    }
                }
                _ => (Events::AckNewAddress_E10, frame),
            },
            TransportServiceInd::NAK(frame) => match &self.src_addr {
                Some(addr) if addr == &frame.src_addr() => {
                    if frame.tpci_seq() == self.seq_no_send {
                        if self.rep_count < Self::MAX_REP_COUNT {
                            (Events::Nak_E12, frame)
                        } else {
                            (Events::Nak_E13, frame)
                        }
                    } else {
                        (Events::Nak_E11, frame)
                    }
                }
                _ => (Events::NakNewAddress_E14, frame),
            },
            _ => {
                error!("Invalid IND");
                self::panic!("");
            }
        };
        self.handle_event(event, frame, network).await
    }

    async fn handle_event(
        &mut self,
        event: Events,
        frame: Frame,
        network: &NetworkLayer,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        match (event, &self.state) {
            (Events::ConnectReqSameAddress_E00, States::Closed) => {
                self.state = States::OpenIdle;
                self.new_connection_A1(frame)
            }

            (Events::ConnectReqSameAddress_E00, _) => {
                self.state = States::Closed;
                self.disconnect_A6(frame, network).await
            }
            (Events::ConnectReqNewAddress_E01, States::Closed) => {
                self.state = States::OpenIdle;
                self.new_connection_A1(frame)
            }
            (Events::ConnectReqNewAddress_E01, _) => self.reject_A10(frame, network).await,
            (Events::DisconnectReqSameAddress_E02, States::Closed) => Ok(None),
            (Events::DisconnectReqSameAddress_E02, _) => {
                self.state = States::Closed;
                self.notify_disconect_A5(frame)
            }
            (Events::DisconnectReqNewAddress_E03, _) => Ok(None),
            (Events::DataConnected_E04, States::Closed) => self.reject_A10(frame, network).await,
            (Events::DataConnected_E04, _) => self.ack_data_A2(frame, network).await,
            (Events::DataConnected_E05, States::Closed) => self.reject_A10(frame, network).await,
            (Events::DataConnected_E05, _) => self.ack_A3(frame, network).await,
            (_, _) => Ok(None),
        }
    }

    async fn send_response_frame(
        &self,
        dst_addr: IndividualAddress,
        seq: u8,
        network: &NetworkLayer,
        tpci: u8,
    ) -> Result<(), FrameError> {
        let mut frame = StandardFrame::new(StandardFrame::MIN_FRAME_SIZE)?;
        frame.set_priority(Priority::System);
        frame.set_dst_addr(&Address::Individual(dst_addr));
        frame.set_src_addr(&crate::settings::ADDRESS);
        frame.set_hop_count(7);
        frame.set_tpci(TpciBits::Eight, tpci);
        frame.set_tpci_seq(seq);
        network
            .send(NetworkServiceReq::DataIndividual(Frame::Standard(frame)))
            .await;
        Ok(())
    }

    fn new_connection_A1(
        &mut self,
        frame: Frame,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        // A1
        self.seq_no_recv = 0;
        self.seq_no_send = 0;
        self.src_addr = Some(frame.src_addr());
        self.connection_timeout = Some(Ticker::every(Duration::from_secs(
            Self::CONNECTION_TIMEOUT_SEC,
        )));
        Ok(Some(TransportServiceInd::Connect(frame)))
    }

    async fn ack_data_A2(
        &mut self,
        frame: Frame,
        network: &NetworkLayer,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        // A2
        self.send_response_frame(frame.src_addr(), self.seq_no_recv, network, 0xc2)
            .await?;
        self.connection_timeout = Some(Ticker::every(Duration::from_secs(
            Self::CONNECTION_TIMEOUT_SEC,
        )));
        self.seq_no_recv = self.seq_no_recv + 1;
        Ok(Some(TransportServiceInd::DataConnected(frame)))
    }

    async fn ack_A3(
        &mut self,
        frame: Frame,
        network: &NetworkLayer,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        // A3
        self.send_response_frame(frame.src_addr(), frame.tpci_seq(), network, 0xc2)
            .await?;
        self.connection_timeout = Some(Ticker::every(Duration::from_secs(
            Self::CONNECTION_TIMEOUT_SEC,
        )));
        Ok(None)
    }

    async fn nak_A4(
        &mut self,
        frame: Frame,
        network: &NetworkLayer,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        self.send_response_frame(frame.src_addr(), frame.tpci_seq(), network, 0xc3)
            .await?;
        self.connection_timeout = None;
        Ok(None)
    }

    fn notify_disconect_A5(
        &mut self,
        frame: Frame,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        //A5
        self.connection_timeout = None;
        Ok(Some(TransportServiceInd::Disconnect(frame)))
    }

    async fn disconnect_A6(
        &mut self,
        frame: Frame,
        network: &NetworkLayer,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        // A6
        self.send_response_frame(frame.src_addr(), 0, network, 0x81)
            .await?;
        self.connection_timeout = None;
        Ok(Some(TransportServiceInd::Disconnect(frame)))
    }

    async fn send_data_A7(
        &mut self,
        mut frame: Frame,
        network: &NetworkLayer,
    ) -> Result<(), FrameError> {
        frame.set_dst_addr(&Address::Individual(unwrap!(self.src_addr.clone())));
        frame.set_src_addr(&crate::settings::ADDRESS);
        frame.set_hop_count(7);
        frame.set_tpci(TpciBits::Six, 0x10);
        frame.set_tpci_seq(self.seq_no_send);
        self.stored_frame = Some(Frame::try_from(&frame)?);
        network.send(NetworkServiceReq::DataIndividual(frame)).await;
        self.rep_count = 0;
        self.connection_timeout = Some(Ticker::every(Duration::from_secs(
            Self::CONNECTION_TIMEOUT_SEC,
        )));
        // Start ACK timer?
        Ok(())
    }

    async fn confirm_data_A8(&mut self, frame: Frame) -> Result<(), FrameError> {
        // Stop ACK timer
        self.seq_no_send += 1;
        // Send CON
        self.connection_timeout = Some(Ticker::every(Duration::from_secs(
            Self::CONNECTION_TIMEOUT_SEC,
        )));
        Ok(())
    }

    async fn repeat_data_A9(&mut self, network: &NetworkLayer) -> Result<(), FrameError> {
        let stored_frame = Frame::try_from(unwrap!(self.stored_frame.as_ref()))?;
        network
            .send(NetworkServiceReq::DataIndividual(stored_frame))
            .await;
        self.rep_count += 1;
        self.connection_timeout = Some(Ticker::every(Duration::from_secs(
            Self::CONNECTION_TIMEOUT_SEC,
        )));
        // Start ACK timer
        Ok(())
    }

    async fn reject_A10(
        &mut self,
        frame: Frame,
        network: &NetworkLayer,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        // A10
        self.send_response_frame(frame.src_addr(), 0, network, 0x81)
            .await?;
        Ok(None)
    }

    //async fn connect_A12(&mut self, )

    async fn connection_timeout(&mut self) -> Result<Option<TransportServiceInd>, FrameError> {
        Ok(None)
    }
}
