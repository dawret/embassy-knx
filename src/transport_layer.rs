use crate::bus::ConStatus;
use crate::frame::*;
use crate::group_address_table::{GroupAddressTable, Tsap};
use crate::network_layer::{NetworkLayer, NetworkServiceInd, NetworkServiceReq};
use core::cell::RefCell;
use embassy_time::{Duration, Instant, Timer};

pub struct TransportLayer {
    network: NetworkLayer,
    group_addresses: GroupAddressTable,
    connection: RefCell<Connection>,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TransportServiceInd {
    DataBroadcast(Frame),
    DataSystemBroadcast(Frame),
    DataGroup {
        tsap: Tsap,
        frame: Frame,
    },
    DataTagGroup(Frame),
    DataIndividual(Frame),
    DataConnected(Frame),
    /// T_Data_Connected.con: the frame of the request, acknowledged by the partner.
    DataConnectedCon(Frame),
    Connect(Frame),
    Disconnect,
}

pub enum TransportServiceReq {
    DataGroup { tsap: Tsap, frame: Frame },
}

const T_CONNECT: u8 = 0x80;
const T_DISCONNECT: u8 = 0x81;
const T_ACK: u8 = 0xC2;
const T_NAK: u8 = 0xC3;

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum ConnectionPdu {
    Connect,
    Disconnect,
    DataConnected,
    Ack,
    Nak,
}

// The event loop handles one event at a time, so a borrow held across an await is never contended.
#[allow(clippy::await_holding_refcell_ref)]
impl TransportLayer {
    pub fn new(network: NetworkLayer, group_addresses: GroupAddressTable) -> Self {
        Self {
            network,
            group_addresses,
            connection: RefCell::new(Connection::new()),
        }
    }

    /// Returns the frame with its L_Data.con, or `None` if the TSAP is not in the group
    /// address table.
    pub async fn send(&self, req: TransportServiceReq) -> Option<(Frame, ConStatus)> {
        match req {
            TransportServiceReq::DataGroup { tsap, mut frame } => {
                let address = self.group_addresses.address(tsap)?;
                frame.set_dst_addr(&Address::Group(address));
                frame.set_src_addr(&crate::settings::ADDRESS);
                frame.set_hop_count(7);
                frame.set_tpci(TpciBits::Six, 0x0);
                Some(self.network.send(NetworkServiceReq::DataGroup(frame)).await)
            }
        }
    }

    /// T_Data_Connected.req. The confirmation arrives later from `handle_frame` as
    /// `DataConnectedCon`. A `Disconnect`, from here or later, ends the request instead.
    /// Only one request may be outstanding.
    pub async fn send_connected(
        &self,
        frame: Frame,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        Ok(self
            .connection
            .borrow_mut()
            .handle_event(Event::DataReq_E15(frame), &self.network)
            .await)
    }

    pub async fn receive_ind(
        &self,
        ind: NetworkServiceInd,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        match ind {
            NetworkServiceInd::DataIndividual(frame) => {
                let tpci = frame.tpci(TpciBits::Eight);
                let pdu = if tpci >> 2 == 0 {
                    return Ok(Some(TransportServiceInd::DataIndividual(frame)));
                } else if tpci >> 6 == 1 {
                    ConnectionPdu::DataConnected
                } else if tpci == T_CONNECT {
                    ConnectionPdu::Connect
                } else if tpci == T_DISCONNECT {
                    ConnectionPdu::Disconnect
                } else if tpci & 0xC3 == T_ACK {
                    ConnectionPdu::Ack
                } else if tpci & 0xC3 == T_NAK {
                    ConnectionPdu::Nak
                } else {
                    return Err(FrameError::InvalidTpdu(tpci));
                };
                info!("{}", pdu);
                Ok(self
                    .connection
                    .borrow_mut()
                    .receive(pdu, frame, &self.network)
                    .await)
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
                    0 => {
                        let Address::Group(address) = frame.dst_addr() else {
                            unreachable!()
                        };
                        Ok(self
                            .group_addresses
                            .tsap(&address)
                            .map(|tsap| TransportServiceInd::DataGroup { tsap, frame }))
                    }
                    1 => Ok(Some(TransportServiceInd::DataTagGroup(frame))),
                    _ => Err(FrameError::InvalidTpdu(tpci)),
                }
            }
            NetworkServiceInd::DataSystemBroadcast(frame) => {
                Ok(Some(TransportServiceInd::DataSystemBroadcast(frame)))
            }
        }
    }

    pub async fn next_frame(&self) -> Frame {
        self.network.next_frame().await
    }

    pub async fn handle_frame(
        &self,
        frame: Frame,
    ) -> Result<Option<TransportServiceInd>, FrameError> {
        match self.network.receive(frame) {
            Some(ind) => self.receive_ind(ind).await,
            None => Ok(None),
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.connection.borrow().deadline()
    }

    pub async fn handle_timeout(&self) -> Result<Option<TransportServiceInd>, FrameError> {
        Ok(self
            .connection
            .borrow_mut()
            .handle_timeout(&self.network)
            .await)
    }
}

/// Completes at `deadline`, never if there is none.
pub async fn at(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => Timer::at(deadline).await,
        None => core::future::pending().await,
    }
}

/// The events of the KNX connection-oriented transport layer state machine (3/3/4 §5.2).
/// The N_Data_Individual.con events E19–E24 are not modelled: a server does nothing on them,
/// and every send already waits for its confirmation. E25 and E26 are client only.
#[allow(non_camel_case_types)]
enum Event {
    ConnectSameAddress_E00(Frame),
    ConnectNewAddress_E01(Frame),
    DisconnectSameAddress_E02,
    DisconnectNewAddress_E03,
    DataExpectedSeq_E04(Frame),
    DataRepeatedSeq_E05(Frame),
    DataWrongSeq_E06(Frame),
    DataNewAddress_E07(Frame),
    AckExpectedSeq_E08(Frame),
    AckWrongSeq_E09(Frame),
    AckNewAddress_E10(Frame),
    NakWrongSeq_E11(Frame),
    NakRepeat_E12(Frame),
    NakMaxRepeats_E13(Frame),
    NakNewAddress_E14(Frame),
    DataReq_E15(Frame),
    ConnectionTimeout_E16,
    AckTimeoutRepeat_E17,
    AckTimeoutMaxRepeats_E18,
}

fn next_seq(seq: u8) -> u8 {
    seq.wrapping_add(1) & 0xF
}

fn prev_seq(seq: u8) -> u8 {
    seq.wrapping_sub(1) & 0xF
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    OpenIdle,
    OpenWait,
}

struct Connection {
    state: State,
    seq_no_send: u8,
    seq_no_recv: u8,
    rep_count: u8,
    connection_address: Option<IndividualAddress>,
    connection_timeout: Option<Instant>,
    ack_timeout: Option<Instant>,
    stored_frame: Option<Frame>,
}

// Method suffixes are the action numbers (A1..A10) of the KNX transport layer state machine.
#[allow(non_snake_case)]
impl Connection {
    const MAX_REP_COUNT: u8 = 3;
    // Shortened in unit tests, which run in real time under a 2 s limit.
    const CONNECTION_TIMEOUT: Duration = if cfg!(test) {
        Duration::from_millis(400)
    } else {
        Duration::from_secs(6)
    };
    const ACK_TIMEOUT: Duration = if cfg!(test) {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(3)
    };

    fn new() -> Self {
        Connection {
            state: State::Closed,
            seq_no_send: 0,
            seq_no_recv: 0,
            rep_count: 0,
            connection_address: None,
            connection_timeout: None,
            ack_timeout: None,
            stored_frame: None,
        }
    }

    fn deadline(&self) -> Option<Instant> {
        match (self.connection_timeout, self.ack_timeout) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    async fn receive(
        &mut self,
        pdu: ConnectionPdu,
        frame: Frame,
        network: &NetworkLayer,
    ) -> Option<TransportServiceInd> {
        let from_partner = self.connection_address == Some(frame.src_addr());
        let seq = frame.tpci_seq();
        let event = match pdu {
            ConnectionPdu::Connect if from_partner => Event::ConnectSameAddress_E00(frame),
            ConnectionPdu::Connect => Event::ConnectNewAddress_E01(frame),
            ConnectionPdu::Disconnect if from_partner => Event::DisconnectSameAddress_E02,
            ConnectionPdu::Disconnect => Event::DisconnectNewAddress_E03,
            ConnectionPdu::DataConnected if !from_partner => Event::DataNewAddress_E07(frame),
            ConnectionPdu::DataConnected if seq == self.seq_no_recv => {
                Event::DataExpectedSeq_E04(frame)
            }
            ConnectionPdu::DataConnected if seq == prev_seq(self.seq_no_recv) => {
                Event::DataRepeatedSeq_E05(frame)
            }
            ConnectionPdu::DataConnected => Event::DataWrongSeq_E06(frame),
            ConnectionPdu::Ack if !from_partner => Event::AckNewAddress_E10(frame),
            ConnectionPdu::Ack if seq == self.seq_no_send => Event::AckExpectedSeq_E08(frame),
            ConnectionPdu::Ack => Event::AckWrongSeq_E09(frame),
            ConnectionPdu::Nak if !from_partner => Event::NakNewAddress_E14(frame),
            ConnectionPdu::Nak if seq != self.seq_no_send => Event::NakWrongSeq_E11(frame),
            ConnectionPdu::Nak if self.rep_count < Self::MAX_REP_COUNT => {
                Event::NakRepeat_E12(frame)
            }
            ConnectionPdu::Nak => Event::NakMaxRepeats_E13(frame),
        };
        self.handle_event(event, network).await
    }

    async fn handle_timeout(&mut self, network: &NetworkLayer) -> Option<TransportServiceInd> {
        let now = Instant::now();
        let event = if self.ack_timeout.is_some_and(|t| t <= now) {
            if self.rep_count < Self::MAX_REP_COUNT {
                Event::AckTimeoutRepeat_E17
            } else {
                Event::AckTimeoutMaxRepeats_E18
            }
        } else if self.connection_timeout.is_some_and(|t| t <= now) {
            Event::ConnectionTimeout_E16
        } else {
            return None;
        };
        self.handle_event(event, network).await
    }

    /// Style 1 transition table (3/3/4 §5.4.1), for a device that accepts connections.
    async fn handle_event(
        &mut self,
        event: Event,
        network: &NetworkLayer,
    ) -> Option<TransportServiceInd> {
        use Event::*;
        use State::*;
        match (event, self.state) {
            (ConnectSameAddress_E00(frame) | ConnectNewAddress_E01(frame), Closed) => {
                self.new_connection_A1(frame)
            }
            (ConnectSameAddress_E00(_), _) => self.disconnect_A6(network).await,
            (ConnectNewAddress_E01(frame), _) => self.reject_A10(&frame, network).await,

            (DisconnectSameAddress_E02, OpenIdle | OpenWait) => self.notify_disconnect_A5(),
            (DisconnectSameAddress_E02 | DisconnectNewAddress_E03, _) => None,

            (
                DataExpectedSeq_E04(frame)
                | DataRepeatedSeq_E05(frame)
                | DataWrongSeq_E06(frame)
                | AckExpectedSeq_E08(frame)
                | AckWrongSeq_E09(frame)
                | NakWrongSeq_E11(frame)
                | NakRepeat_E12(frame)
                | NakMaxRepeats_E13(frame),
                Closed,
            )
            | (
                DataNewAddress_E07(frame) | AckNewAddress_E10(frame) | NakNewAddress_E14(frame),
                _,
            ) => self.reject_A10(&frame, network).await,

            (DataExpectedSeq_E04(frame), _) => self.ack_data_A2(frame, network).await,
            (DataRepeatedSeq_E05(frame), _) => self.ack_A3(&frame, network).await,
            (DataWrongSeq_E06(frame), _) => self.nak_A4(&frame, network).await,

            (AckExpectedSeq_E08(_), OpenWait) => self.confirm_data_A8(),
            (NakRepeat_E12(_), OpenWait) | (AckTimeoutRepeat_E17, OpenWait) => {
                self.repeat_data_A9(network).await
            }

            (DataReq_E15(_), Closed) => self.notify_disconnect_A5(),
            (DataReq_E15(frame), OpenIdle) => self.send_data_A7(frame, network).await,

            (
                AckExpectedSeq_E08(_)
                | AckWrongSeq_E09(_)
                | NakWrongSeq_E11(_)
                | NakRepeat_E12(_)
                | NakMaxRepeats_E13(_)
                | DataReq_E15(_),
                _,
            )
            | (ConnectionTimeout_E16, OpenIdle | OpenWait)
            | (AckTimeoutMaxRepeats_E18, OpenWait) => self.disconnect_A6(network).await,

            (ConnectionTimeout_E16 | AckTimeoutRepeat_E17 | AckTimeoutMaxRepeats_E18, _) => None,
        }
    }

    /// A control frame that cannot be allocated is treated like a lost frame, which the
    /// state machine recovers from through repetitions or timeouts.
    async fn send_control(
        &self,
        dst_addr: IndividualAddress,
        tpci: u8,
        seq: u8,
        network: &NetworkLayer,
    ) {
        let mut frame = match Frame::new(FrameType::Standard, FrameType::Standard.min_length()) {
            Ok(frame) => frame,
            Err(e) => {
                error!("Cannot send T-PDU {:x}: {}", tpci, e);
                return;
            }
        };
        frame.set_priority(Priority::System);
        frame.set_dst_addr(&Address::Individual(dst_addr));
        frame.set_src_addr(&crate::settings::ADDRESS);
        frame.set_hop_count(7);
        frame.set_tpci(TpciBits::Eight, tpci);
        frame.set_tpci_seq(seq);
        network.send(NetworkServiceReq::DataIndividual(frame)).await;
    }

    fn restart_connection_timeout(&mut self) {
        self.connection_timeout = Some(Instant::now() + Self::CONNECTION_TIMEOUT);
    }

    fn start_ack_timeout(&mut self) {
        self.ack_timeout = Some(Instant::now() + Self::ACK_TIMEOUT);
    }

    fn close(&mut self) {
        self.state = State::Closed;
        self.connection_timeout = None;
        self.ack_timeout = None;
        self.stored_frame = None;
    }

    fn new_connection_A1(&mut self, frame: Frame) -> Option<TransportServiceInd> {
        self.state = State::OpenIdle;
        self.connection_address = Some(frame.src_addr());
        self.seq_no_send = 0;
        self.seq_no_recv = 0;
        self.restart_connection_timeout();
        Some(TransportServiceInd::Connect(frame))
    }

    async fn ack_data_A2(
        &mut self,
        frame: Frame,
        network: &NetworkLayer,
    ) -> Option<TransportServiceInd> {
        self.send_control(frame.src_addr(), T_ACK, self.seq_no_recv, network)
            .await;
        self.seq_no_recv = next_seq(self.seq_no_recv);
        self.restart_connection_timeout();
        Some(TransportServiceInd::DataConnected(frame))
    }

    async fn ack_A3(
        &mut self,
        frame: &Frame,
        network: &NetworkLayer,
    ) -> Option<TransportServiceInd> {
        self.send_control(frame.src_addr(), T_ACK, frame.tpci_seq(), network)
            .await;
        self.restart_connection_timeout();
        None
    }

    async fn nak_A4(
        &mut self,
        frame: &Frame,
        network: &NetworkLayer,
    ) -> Option<TransportServiceInd> {
        self.send_control(frame.src_addr(), T_NAK, frame.tpci_seq(), network)
            .await;
        self.restart_connection_timeout();
        None
    }

    fn notify_disconnect_A5(&mut self) -> Option<TransportServiceInd> {
        self.close();
        Some(TransportServiceInd::Disconnect)
    }

    async fn disconnect_A6(&mut self, network: &NetworkLayer) -> Option<TransportServiceInd> {
        self.close();
        let partner = unwrap!(self.connection_address);
        self.send_control(partner, T_DISCONNECT, 0, network).await;
        Some(TransportServiceInd::Disconnect)
    }

    async fn send_data_A7(
        &mut self,
        mut frame: Frame,
        network: &NetworkLayer,
    ) -> Option<TransportServiceInd> {
        self.state = State::OpenWait;
        frame.set_dst_addr(&Address::Individual(unwrap!(self.connection_address)));
        frame.set_src_addr(&crate::settings::ADDRESS);
        frame.set_hop_count(7);
        frame.set_tpci(TpciBits::Six, 0x10);
        frame.set_tpci_seq(self.seq_no_send);
        let (frame, _) = network.send(NetworkServiceReq::DataIndividual(frame)).await;
        self.stored_frame = Some(frame);
        self.rep_count = 0;
        self.start_ack_timeout();
        self.restart_connection_timeout();
        None
    }

    fn confirm_data_A8(&mut self) -> Option<TransportServiceInd> {
        self.state = State::OpenIdle;
        self.ack_timeout = None;
        self.seq_no_send = next_seq(self.seq_no_send);
        self.restart_connection_timeout();
        self.stored_frame
            .take()
            .map(TransportServiceInd::DataConnectedCon)
    }

    async fn repeat_data_A9(&mut self, network: &NetworkLayer) -> Option<TransportServiceInd> {
        let frame = unwrap!(self.stored_frame.take());
        let (frame, _) = network.send(NetworkServiceReq::DataIndividual(frame)).await;
        self.stored_frame = Some(frame);
        self.rep_count += 1;
        self.start_ack_timeout();
        self.restart_connection_timeout();
        None
    }

    async fn reject_A10(
        &mut self,
        frame: &Frame,
        network: &NetworkLayer,
    ) -> Option<TransportServiceInd> {
        self.send_control(frame.src_addr(), T_DISCONNECT, 0, network)
            .await;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{receive, run, settle, StackHarness};

    const ME: [u8; 2] = [0x11, 0x78];
    const PEER: [u8; 2] = [0x11, 0xFA];
    const OTHER: [u8; 2] = [0x11, 0xFB];

    fn individual(src: [u8; 2], npci: u8, tpdu: &[u8]) -> Vec<u8> {
        [&[0xB0, src[0], src[1], ME[0], ME[1], npci], tpdu].concat()
    }
    fn t_connect(src: [u8; 2]) -> Vec<u8> {
        individual(src, 0x60, &[0x80])
    }
    fn t_disconnect(src: [u8; 2]) -> Vec<u8> {
        individual(src, 0x60, &[0x81])
    }
    fn t_data_connected(src: [u8; 2], seq: u8) -> Vec<u8> {
        individual(src, 0x61, &[0x40 | seq << 2 | 0x03, 0x00])
    }
    fn t_ack(src: [u8; 2], seq: u8) -> Vec<u8> {
        individual(src, 0x60, &[0xC2 | seq << 2])
    }
    fn t_nak(src: [u8; 2], seq: u8) -> Vec<u8> {
        individual(src, 0x60, &[0xC3 | seq << 2])
    }
    fn sent_control(dst: [u8; 2], tpci: u8) -> Vec<u8> {
        vec![0xB0, ME[0], ME[1], dst[0], dst[1], 0x70, tpci]
    }
    /// A_DeviceDescriptor_Read, before the transport layer addresses it.
    fn device_descriptor_read() -> Frame {
        crate::test_util::frame(&[0xB0, 0, 0, 0, 0, 0x61, 0x03, 0x00])
    }
    fn sent_data(seq: u8) -> Vec<u8> {
        vec![
            0xB0,
            ME[0],
            ME[1],
            PEER[0],
            PEER[1],
            0x71,
            0x43 | seq << 2,
            0x00,
        ]
    }

    async fn connect(h: &StackHarness, tl: &TransportLayer) {
        h.inject(&t_connect(PEER));
        let ind = receive(tl).await;
        assert!(
            matches!(ind, Ok(TransportServiceInd::Connect(_))),
            "{ind:?}"
        );
    }

    #[test]
    fn group_frame_is_indicated() {
        let (h, tl) = StackHarness::new();
        h.inject(&[0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE1, 0x00, 0x00]);
        let ind = run(h.run(receive(&tl)));
        let tsap = Tsap::new(1);
        assert!(
            matches!(ind, Ok(TransportServiceInd::DataGroup { tsap: t, .. }) if t == tsap),
            "{ind:?}"
        );
    }

    #[test]
    fn group_frame_to_unknown_address_is_dropped() {
        let (h, tl) = StackHarness::new();
        h.inject(&[0xBC, 0x11, 0x0A, 0x09, 0x63, 0xE1, 0x00, 0x00]);
        h.inject(&[0xBC, 0x11, 0x0A, 0x00, 0x00, 0xE1, 0x00, 0x00]);
        let ind = run(h.run(receive(&tl)));
        assert!(
            matches!(ind, Ok(TransportServiceInd::DataBroadcast(_))),
            "{ind:?}"
        );
    }

    #[test]
    fn group_frame_is_sent_to_the_address_of_its_tsap() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            let frame = Frame::group_read(Priority::Low).unwrap();
            let req = TransportServiceReq::DataGroup {
                tsap: Tsap::new(1),
                frame,
            };
            let (_, status) = tl.send(req).await.unwrap();
            assert_eq!(status, ConStatus::Ok);
            assert_eq!(
                h.sent().await,
                [0xBC, ME[0], ME[1], 0x09, 0x62, 0xF1, 0x00, 0x00]
            );

            let frame = Frame::group_read(Priority::Low).unwrap();
            let req = TransportServiceReq::DataGroup {
                tsap: Tsap::new(2),
                frame,
            };
            assert!(tl.send(req).await.is_none());
        }));
    }

    #[test]
    fn broadcast_frame_is_indicated() {
        let (h, tl) = StackHarness::new();
        h.inject(&[0xBC, 0x11, 0x0A, 0x00, 0x00, 0xE1, 0x00, 0x00]);
        let ind = run(h.run(receive(&tl)));
        assert!(
            matches!(ind, Ok(TransportServiceInd::DataBroadcast(_))),
            "{ind:?}"
        );
    }

    #[test]
    fn unknown_control_tpdu_is_rejected() {
        let (h, tl) = StackHarness::new();
        h.inject(&individual(PEER, 0x60, &[0x82]));
        let ind = run(h.run(receive(&tl)));
        assert!(matches!(ind, Err(FrameError::InvalidTpdu(0x82))), "{ind:?}");
    }

    #[test]
    fn connect_then_data_is_acked() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::Connect(_))),
                "{ind:?}"
            );

            h.inject(&t_data_connected(PEER, 0));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::DataConnected(_))),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));
        }));
    }

    #[test]
    fn repeated_data_is_acked_but_not_indicated() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            receive(&tl).await.unwrap();
            h.inject(&t_data_connected(PEER, 0));
            receive(&tl).await.unwrap();
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));

            h.inject(&t_data_connected(PEER, 0));
            h.inject(&t_disconnect(PEER));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::Disconnect)),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));
        }));
    }

    #[test]
    fn sequence_number_wraps_after_15() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            receive(&tl).await.unwrap();
            for seq in (0..16).chain([0, 1]) {
                h.inject(&t_data_connected(PEER, seq));
                let ind = receive(&tl).await;
                assert!(
                    matches!(ind, Ok(TransportServiceInd::DataConnected(_))),
                    "seq {seq}: {ind:?}"
                );
                assert_eq!(
                    h.sent().await,
                    sent_control(PEER, 0xC2 | seq << 2),
                    "seq {seq}"
                );
            }
        }));
    }

    #[test]
    fn out_of_sequence_data_is_nakked() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            h.inject(&t_data_connected(PEER, 3));
            h.inject(&t_data_connected(PEER, 0));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::DataConnected(_))),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(PEER, 0xC3 | 3 << 2));
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));
        }));
    }

    #[test]
    fn connection_times_out() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::Disconnect)),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(PEER, 0x81));
        }));
    }

    #[test]
    fn nak_keeps_the_connection_timeout_running() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            h.inject(&t_data_connected(PEER, 3));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::Disconnect)),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(PEER, 0xC3 | 3 << 2));
            assert_eq!(h.sent().await, sent_control(PEER, 0x81));
        }));
    }

    #[test]
    fn connect_from_the_partner_closes_the_connection() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            h.inject(&t_connect(PEER));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::Disconnect)),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(PEER, 0x81));
        }));
    }

    #[test]
    fn unexpected_ack_or_nak_closes_the_connection() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            for pdu in [t_ack(PEER, 0), t_nak(PEER, 0), t_nak(PEER, 1)] {
                connect(h, &tl).await;
                h.inject(&pdu);
                let ind = receive(&tl).await;
                assert!(
                    matches!(ind, Ok(TransportServiceInd::Disconnect)),
                    "{pdu:x?}: {ind:?}"
                );
                assert_eq!(h.sent().await, sent_control(PEER, 0x81), "{pdu:x?}");
            }
        }));
    }

    #[test]
    fn pdus_from_another_device_are_rejected() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            h.inject(&t_data_connected(OTHER, 0));
            h.inject(&t_ack(OTHER, 0));
            h.inject(&t_nak(OTHER, 0));
            h.inject(&t_data_connected(PEER, 0));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::DataConnected(_))),
                "{ind:?}"
            );
            for _ in 0..3 {
                assert_eq!(h.sent().await, sent_control(OTHER, 0x81));
            }
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));
        }));
    }

    #[test]
    fn pdus_without_a_connection_are_rejected() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_data_connected(PEER, 5));
            h.inject(&t_ack(PEER, 0));
            h.inject(&t_nak(PEER, 0));
            h.inject(&[0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE1, 0x00, 0x00]);
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::DataGroup { .. })),
                "{ind:?}"
            );
            for _ in 0..3 {
                assert_eq!(h.sent().await, sent_control(PEER, 0x81));
            }
        }));
    }

    #[test]
    fn second_connection_is_rejected() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            receive(&tl).await.unwrap();

            h.inject(&t_connect(OTHER));
            h.inject(&t_data_connected(PEER, 0));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::DataConnected(_))),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(OTHER, 0x81));
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));
        }));
    }

    #[test]
    fn data_after_disconnect_is_rejected() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            h.inject(&t_connect(PEER));
            receive(&tl).await.unwrap();
            h.inject(&t_disconnect(PEER));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::Disconnect)),
                "{ind:?}"
            );

            h.inject(&t_data_connected(PEER, 0));
            h.inject(&[0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE1, 0x00, 0x00]);
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::DataGroup { .. })),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(PEER, 0x81));
            settle().await;
            assert!(h.nothing_sent());
        }));
    }

    #[test]
    fn data_is_sent_and_confirmed() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            for seq in [0, 1] {
                let ind = tl.send_connected(device_descriptor_read()).await;
                assert!(matches!(ind, Ok(None)), "{ind:?}");
                assert_eq!(h.sent().await, sent_data(seq));
                h.inject(&t_ack(PEER, seq));
                let ind = receive(&tl).await;
                assert!(
                    matches!(&ind, Ok(TransportServiceInd::DataConnectedCon(f)) if f.tpci_seq() == seq),
                    "seq {seq}: {ind:?}"
                );
            }
        }));
    }

    #[test]
    fn data_is_received_while_waiting_for_ack() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            tl.send_connected(device_descriptor_read()).await.unwrap();
            assert_eq!(h.sent().await, sent_data(0));

            h.inject(&t_data_connected(PEER, 0));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::DataConnected(_))),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(PEER, 0xC2));

            h.inject(&t_ack(PEER, 0));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::DataConnectedCon(_))),
                "{ind:?}"
            );
        }));
    }

    #[test]
    fn ack_from_another_device_does_not_confirm() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            tl.send_connected(device_descriptor_read()).await.unwrap();
            assert_eq!(h.sent().await, sent_data(0));

            h.inject(&t_ack(OTHER, 0));
            h.inject(&t_ack(PEER, 0));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::DataConnectedCon(_))),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(OTHER, 0x81));
        }));
    }

    #[test]
    fn nak_repeats_data_until_the_connection_closes() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            tl.send_connected(device_descriptor_read()).await.unwrap();
            for _ in 0..4 {
                h.inject(&t_nak(PEER, 0));
            }
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::Disconnect)),
                "{ind:?}"
            );
            for attempt in 0..4 {
                assert_eq!(h.sent().await, sent_data(0), "attempt {attempt}");
            }
            assert_eq!(h.sent().await, sent_control(PEER, 0x81));
        }));
    }

    #[test]
    fn ack_timeout_repeats_data_until_the_connection_closes() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            tl.send_connected(device_descriptor_read()).await.unwrap();
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::Disconnect)),
                "{ind:?}"
            );
            for attempt in 0..4 {
                assert_eq!(h.sent().await, sent_data(0), "attempt {attempt}");
            }
            assert_eq!(h.sent().await, sent_control(PEER, 0x81));
        }));
    }

    #[test]
    fn wrong_ack_closes_the_connection() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            tl.send_connected(device_descriptor_read()).await.unwrap();
            h.inject(&t_ack(PEER, 1));
            let ind = receive(&tl).await;
            assert!(
                matches!(ind, Ok(TransportServiceInd::Disconnect)),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_data(0));
            assert_eq!(h.sent().await, sent_control(PEER, 0x81));
        }));
    }

    #[test]
    fn sending_without_a_connection_fails() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            let ind = tl.send_connected(device_descriptor_read()).await;
            assert!(
                matches!(ind, Ok(Some(TransportServiceInd::Disconnect))),
                "{ind:?}"
            );
            settle().await;
            assert!(h.nothing_sent());
        }));
    }

    #[test]
    fn sending_before_the_confirmation_closes_the_connection() {
        let (h, tl) = StackHarness::new();
        run(h.run(async {
            connect(h, &tl).await;
            tl.send_connected(device_descriptor_read()).await.unwrap();
            assert_eq!(h.sent().await, sent_data(0));
            let ind = tl.send_connected(device_descriptor_read()).await;
            assert!(
                matches!(ind, Ok(Some(TransportServiceInd::Disconnect))),
                "{ind:?}"
            );
            assert_eq!(h.sent().await, sent_control(PEER, 0x81));
        }));
    }
}
