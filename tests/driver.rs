mod common;

use common::*;
use embassy_knx::bus::{Bus, ConStatus};
use embassy_knx::frame::{Address, Frame, IndividualAddress};
use embassy_knx::ncn51_driver::NCN51Driver;

const GROUP_VALUE_READ: [u8; 8] = [0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE1, 0x00, 0x00];

fn t_connect(dst: [u8; 2]) -> [u8; 7] {
    [0xB0, 0x11, 0xFA, dst[0], dst[1], 0x60, 0x80]
}

/// A_GroupValue_Response(0x1234) from 1.1.120 to 1/1/98.
const GROUP_VALUE_RESPONSE: [u8; 10] = [0xB4, 0x11, 0x78, 0x09, 0x62, 0xE3, 0x00, 0x40, 0x12, 0x34];

/// Frames built by the stack carry an empty checksum slot.
fn tx_frame(telegram: &[u8]) -> Frame {
    Frame::from_slice(&[telegram, &[0]].concat()).unwrap()
}

fn setup() -> (
    &'static Ncn5130Sim,
    &'static Bus,
    NCN51Driver<SimUart<'static>, NoLed>,
) {
    init_pool();
    let sim = leak(Ncn5130Sim::new());
    let bus = leak(Bus::new());
    (sim, bus, NCN51Driver::new(sim.uart(), NoLed, bus))
}

#[test]
fn receives_group_frame_and_acks_it() {
    let (sim, bus, driver) = setup();
    run(with_background(
        async {
            sim.bus_receive(&GROUP_VALUE_READ).await;
            sim.expect_ack().await;
            let frame = bus.bus_rx.receive().await;
            assert_eq!(frame.data(), with_checksum(&GROUP_VALUE_READ));
        },
        driver.run(),
    ));
}

#[test]
fn acks_only_its_own_individual_address() {
    let (sim, bus, driver) = setup();
    run(with_background(
        async {
            sim.bus_receive(&t_connect([0x11, 0x05])).await;
            sim.bus_receive(&t_connect(ME)).await;
            let me = Address::Individual(IndividualAddress::from_parts(1, 1, 120));
            while bus.bus_rx.receive().await.dst_addr() != me {}
            settle().await;
            assert_eq!(sim.host_bytes_now(), [U_ACK]);
        },
        driver.run(),
    ));
}

#[test]
fn drops_frame_with_bad_checksum() {
    let (sim, bus, driver) = setup();
    run(with_background(
        async {
            let mut corrupt = with_checksum(&GROUP_VALUE_READ);
            *corrupt.last_mut().unwrap() ^= 0x01;
            sim.send_to_host(&corrupt);
            sim.bus_idle().await;
            sim.bus_receive(&GROUP_VALUE_READ).await;
            let frame = bus.bus_rx.receive().await;
            assert_eq!(frame.data(), with_checksum(&GROUP_VALUE_READ));
            assert!(bus.bus_rx.is_empty());
        },
        driver.run(),
    ));
}

#[test]
fn receives_full_length_extended_frame() {
    let (sim, bus, driver) = setup();
    run(with_background(
        async {
            // 263 octets: header (8) + LG 254 + checksum.
            let mut telegram = vec![0x3C, 0xE0, 0x11, 0x0A, 0x09, 0x62, 254];
            telegram.resize(262, 0);
            sim.bus_receive(&telegram).await;
            sim.expect_ack().await;
            let frame = bus.bus_rx.receive().await;
            assert_eq!(frame.data(), with_checksum(&telegram));
        },
        driver.run(),
    ));
}

#[test]
fn skips_unexpected_bytes() {
    let (sim, bus, driver) = setup();
    run(with_background(
        async {
            sim.send_to_host(&[0x07]);
            sim.bus_receive(&GROUP_VALUE_READ).await;
            let frame = bus.bus_rx.receive().await;
            assert_eq!(frame.data(), with_checksum(&GROUP_VALUE_READ));
        },
        driver.run(),
    ));
}

#[test]
fn transmits_frame_and_reports_confirmation() {
    let (sim, bus, driver) = setup();
    let telegram = [0xB0, 0x11, 0x78, 0x11, 0xFA, 0x70, 0xC2];
    run(with_background(
        async {
            bus.bus_tx.send(tx_frame(&telegram)).await;
            assert_eq!(sim.transmit(true).await, with_checksum(&telegram));
            assert_eq!(bus.con.wait().await.1, ConStatus::Ok);

            bus.bus_tx.send(tx_frame(&telegram)).await;
            assert_eq!(sim.transmit(false).await, with_checksum(&telegram));
            assert_eq!(bus.con.wait().await.1, ConStatus::NotOk);
        },
        driver.run(),
    ));
}

#[test]
fn transmits_standard_frame_in_one_write() {
    let (sim, bus, driver) = setup();
    run(with_background(
        async {
            bus.bus_tx.send(tx_frame(&GROUP_VALUE_RESPONSE)).await;
            assert_eq!(
                sim.transmit(true).await,
                with_checksum(&GROUP_VALUE_RESPONSE)
            );
            assert_eq!(bus.con.wait().await.1, ConStatus::Ok);
            assert_eq!(sim.host_writes(), 1);
        },
        driver.run(),
    ));
}

#[test]
fn confirmation_returns_the_sent_frame() {
    let (sim, bus, driver) = setup();
    run(with_background(
        async {
            bus.bus_tx.send(tx_frame(&GROUP_VALUE_RESPONSE)).await;
            sim.transmit(true).await;
            let (frame, status) = bus.con.wait().await;
            assert_eq!(status, ConStatus::Ok);
            assert_eq!(frame.data(), with_checksum(&GROUP_VALUE_RESPONSE));
            settle().await;
            assert_eq!(sim.host_bytes_now(), [], "own echo must not be acked");
        },
        driver.run(),
    ));
}

#[test]
fn confirms_after_repetitions() {
    let (sim, bus, driver) = setup();
    run(with_background(
        async {
            bus.bus_tx.send(tx_frame(&GROUP_VALUE_RESPONSE)).await;
            let telegram = sim.host_request().await;
            sim.send_to_host(&telegram);
            sim.send_to_host(&repetition(&telegram));
            sim.echo_and_confirm(&repetition(&telegram), true);
            let (frame, status) = bus.con.wait().await;
            assert_eq!(status, ConStatus::Ok);
            assert_eq!(frame.data(), telegram);
        },
        driver.run(),
    ));
}

#[test]
fn receives_frame_while_waiting_for_confirmation() {
    let (sim, bus, driver) = setup();
    run(with_background(
        async {
            bus.bus_tx.send(tx_frame(&GROUP_VALUE_RESPONSE)).await;
            let telegram = sim.host_request().await;
            sim.bus_receive(&GROUP_VALUE_READ).await;
            sim.expect_ack().await;
            sim.echo_and_confirm(&telegram, true);
            assert_eq!(
                bus.bus_rx.receive().await.data(),
                with_checksum(&GROUP_VALUE_READ)
            );
            let (frame, status) = bus.con.wait().await;
            assert_eq!(status, ConStatus::Ok);
            assert_eq!(frame.data(), telegram);
        },
        driver.run(),
    ));
}

#[test]
fn transmits_long_extended_frame() {
    let (sim, bus, driver) = setup();
    let mut telegram = vec![0x3C, 0xE0, 0x11, 0x78, 0x09, 0x62, 91, 0x00, 0x80];
    telegram.resize(99, 0x55);
    run(with_background(
        async {
            bus.bus_tx.send(tx_frame(&telegram)).await;
            assert_eq!(sim.transmit(true).await, with_checksum(&telegram));
            assert_eq!(bus.con.wait().await.1, ConStatus::Ok);
        },
        driver.run(),
    ));
}
