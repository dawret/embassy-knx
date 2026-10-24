use crate::frame::*;
use crate::network_layer::{NetworkLayer, NetworkServiceInd, NetworkServiceReq};
use core::cell::RefCell;
use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Instant, Timer};

pub struct TransportLayer {
    network: NetworkLayer,
    connection: RefCell<Connection>,
}

/*pub enum TSAP {

}*/

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    #[allow(dead_code)] // S6: unused until the association table exists.
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
                } else if tpci & 0xc3 == 0xc2 {
                    info!("T_ACK");
                    self.connection
                        .borrow_mut()
                        .handle_ind(TransportServiceInd::ACK(frame), &self.network)
                        .await
                } else if tpci & 0xc3 == 0xc3 {
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
            // Copy the deadline out: no borrow of `connection` may live across the select.
            let deadline = self.connection.borrow().connection_timeout;
            let timeout = async {
                match deadline {
                    Some(deadline) => Timer::at(deadline).await,
                    None => core::future::pending().await,
                }
            };
            let ret = match select(self.network.receive(), timeout).await {
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

/// Sequence numbers are 4 bits wide and wrap around.
fn next_seq(seq: u8) -> u8 {
    seq.wrapping_add(1) & 0xF
}

fn prev_seq(seq: u8) -> u8 {
    seq.wrapping_sub(1) & 0xF
}

#[allow(dead_code)] // OpenWait and Connecting belong to the unwired sending side.
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
    connection_timeout: Option<Instant>,
    #[allow(dead_code)] // Only read by repeat_data_A9, which is not wired up yet.
    stored_frame: Option<Frame>,
}

// Method suffixes are the action numbers (A1..A10) of the KNX transport layer state machine.
#[allow(non_snake_case)]
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
                    val if val == prev_seq(self.seq_no_recv) => (Events::DataConnected_E05, frame),
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
                panic!("Invalid IND");
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
        self.restart_connection_timeout();
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
        self.restart_connection_timeout();
        self.seq_no_recv = next_seq(self.seq_no_recv);
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
        self.restart_connection_timeout();
        Ok(None)
    }

    #[allow(dead_code)] // Not wired into handle_event yet.
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

    #[allow(dead_code)] // Not wired into handle_event yet.
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
        self.restart_connection_timeout();
        // Start ACK timer?
        Ok(())
    }

    #[allow(dead_code)] // Not wired into handle_event yet.
    async fn confirm_data_A8(&mut self, _frame: Frame) -> Result<(), FrameError> {
        // Stop ACK timer
        self.seq_no_send = next_seq(self.seq_no_send);
        // Send CON
        self.restart_connection_timeout();
        Ok(())
    }

    #[allow(dead_code)] // Not wired into handle_event yet.
    async fn repeat_data_A9(&mut self, network: &NetworkLayer) -> Result<(), FrameError> {
        let stored_frame = Frame::try_from(unwrap!(self.stored_frame.as_ref()))?;
        network
            .send(NetworkServiceReq::DataIndividual(stored_frame))
            .await;
        self.rep_count += 1;
        self.restart_connection_timeout();
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

    fn restart_connection_timeout(&mut self) {
        self.connection_timeout =
            Some(Instant::now() + Duration::from_secs(Self::CONNECTION_TIMEOUT_SEC));
    }

    async fn connection_timeout(&mut self) -> Result<Option<TransportServiceInd>, FrameError> {
        // TODO: A6 (disconnect). Clearing the deadline keeps the timeout from firing in a loop.
        self.connection_timeout = None;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{run, settle, StackHarness};

    /// Our address, `settings::ADDRESS` (1.1.120).
    const ME: [u8; 2] = [0x11, 0x78];
    /// The connecting peer, e.g. ETS (1.1.250).
    const PEER: [u8; 2] = [0x11, 0xFA];
    const OTHER: [u8; 2] = [0x11, 0xFB];

    // Telegrams without checksum. Control 0xB0: standard frame, not repeated, system priority.
    fn individual(src: [u8; 2], npci: u8, tpdu: &[u8]) -> Vec<u8> {
        [&[0xB0, src[0], src[1], ME[0], ME[1], npci], tpdu].concat()
    }
    fn t_connect(src: [u8; 2]) -> Vec<u8> {
        individual(src, 0x60, &[0x80])
    }
    fn t_disconnect(src: [u8; 2]) -> Vec<u8> {
        individual(src, 0x60, &[0x81])
    }
    /// T_Data_Connected carrying A_DeviceDescriptor_Read.
    fn t_data_connected(src: [u8; 2], seq: u8) -> Vec<u8> {
        individual(src, 0x61, &[0x40 | seq << 2 | 0x03, 0x00])
    }
    fn t_ack(src: [u8; 2], seq: u8) -> Vec<u8> {
        individual(src, 0x60, &[0xC2 | seq << 2])
    }
    fn t_nak(src: [u8; 2], seq: u8) -> Vec<u8> {
        individual(src, 0x60, &[0xC3 | seq << 2])
    }
    /// A control telegram the stack sends to `dst` (hop count 7).
    fn sent_control(dst: [u8; 2], tpci: u8) -> Vec<u8> {
        vec![0xB0, ME[0], ME[1], dst[0], dst[1], 0x70, tpci]
    }

    #[test]
    fn group_frame_is_indicated() {
        let (h, tl) = StackHarness::new();
        // A_GroupValue_Read from 1.1.10 to 1/1/98.
        h.inject(&[0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE1, 0x00, 0x00]);
        let ind = run(h.run(tl.receive()));
        assert!(matches!(ind, Ok(TransportServiceInd::DataGroup(_))), "{ind:?}");
    }

    #[test]
    fn broadcast_frame_is_indicated() {
        let (h, tl) = StackHarness::new();
        h.inject(&[0xBC, 0x11, 0x0A, 0x00, 0x00, 0xE1, 0x00, 0x00]);
        let ind = run(h.run(tl.receive()));
        assert!(matches!(ind, Ok(TransportServiceInd::DataBroadcast(_))), "{ind:?}");
    }

    #[test]
    fn unknown_control_tpdu_is_rejected() {
        let (h, tl) = StackHarness::new();
        h.inject(&individual(PEER, 0x60, &[0x82]));
        let ind = run(h.run(tl.receive()));
        assert!(matches!(ind, Err(FrameError::InvalidTpdu(0x82))), "{ind:?}");
    }

    #[test]
    fn connect_then_data_is_acked() {
        // B1: any connection-oriented frame used to panic with BorrowMutError.
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            let ind = tl.receive().await;
            assert!(matches!(ind, Ok(TransportServiceInd::Connect(_))), "{ind:?}");

            h.inject(&t_data_connected(PEER, 0));
            let ind = tl.receive().await;
            assert!(matches!(ind, Ok(TransportServiceInd::DataConnected(_))), "{ind:?}");
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));
        }));
    }

    #[test]
    fn repeated_data_is_acked_but_not_indicated() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            tl.receive().await.unwrap();
            h.inject(&t_data_connected(PEER, 0));
            tl.receive().await.unwrap();
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));

            // The peer missed our T_ACK and repeats the frame.
            h.inject(&t_data_connected(PEER, 0));
            h.inject(&t_disconnect(PEER));
            let ind = tl.receive().await;
            assert!(matches!(ind, Ok(TransportServiceInd::Disconnect(_))), "{ind:?}");
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));
        }));
    }

    #[test]
    fn sequence_number_wraps_after_15() {
        // B3
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            tl.receive().await.unwrap();
            for seq in (0..16).chain([0, 1]) {
                h.inject(&t_data_connected(PEER, seq));
                let ind = tl.receive().await;
                assert!(
                    matches!(ind, Ok(TransportServiceInd::DataConnected(_))),
                    "seq {seq}: {ind:?}"
                );
                assert_eq!(h.sent().await, sent_control(PEER, 0xC2 | seq << 2), "seq {seq}");
            }
        }));
    }

    #[test]
    fn out_of_sequence_data_right_after_connect() {
        // B3: `seq_no_recv - 1` underflowed at 0.
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            tl.receive().await.unwrap();
            h.inject(&t_data_connected(PEER, 3));
            h.inject(&t_data_connected(PEER, 0));
            let ind = tl.receive().await;
            assert!(matches!(ind, Ok(TransportServiceInd::DataConnected(_))), "{ind:?}");
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));
        }));
    }

    #[test]
    fn t_ack_and_t_nak_are_recognised() {
        // B2: both were rejected as invalid TPDUs.
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            tl.receive().await.unwrap();
            h.inject(&t_ack(PEER, 0));
            h.inject(&t_nak(PEER, 0));
            h.inject(&t_disconnect(PEER));
            let ind = tl.receive().await;
            assert!(matches!(ind, Ok(TransportServiceInd::Disconnect(_))), "{ind:?}");
        }));
    }

    #[test]
    fn second_connection_is_rejected() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            tl.receive().await.unwrap();

            h.inject(&t_connect(OTHER));
            h.inject(&t_data_connected(PEER, 0));
            let ind = tl.receive().await;
            assert!(matches!(ind, Ok(TransportServiceInd::DataConnected(_))), "{ind:?}");
            assert_eq!(h.sent().await, sent_control(OTHER, 0x81));
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));
        }));
    }

    #[test]
    fn data_after_disconnect_is_rejected() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            tl.receive().await.unwrap();
            h.inject(&t_disconnect(PEER));
            let ind = tl.receive().await;
            assert!(matches!(ind, Ok(TransportServiceInd::Disconnect(_))), "{ind:?}");

            h.inject(&t_data_connected(PEER, 0));
            h.inject(&[0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE1, 0x00, 0x00]);
            let ind = tl.receive().await;
            assert!(matches!(ind, Ok(TransportServiceInd::DataGroup(_))), "{ind:?}");
            assert_eq!(h.sent().await, sent_control(PEER, 0x81));
            settle().await;
            assert!(h.nothing_sent());
        }));
    }
}
