mod common;

use common::*;
use embassy_futures::join::join;
use embassy_futures::poll_once;
use embassy_knx::application_layer::ApplicationLayer;
use embassy_knx::bus::Bus;
use embassy_knx::data_link_layer::DataLinkLayer;
use embassy_knx::data_point::B1;
use embassy_knx::frame::{GroupAddress, Priority};
use embassy_knx::group_address_table::GroupAddressTable;
use embassy_knx::group_object::ConfigFlag::{self, *};
use embassy_knx::group_object::{
    value_bytes, Asap, Descriptor, GroupObjectRam, GroupObjects, TransmissionStatus, ValueType,
};
use embassy_knx::group_object_association_table::{Association, AssociationTable};
use embassy_knx::ncn51_driver::NCN51Driver;
use embassy_knx::network_layer::NetworkLayer;
use embassy_knx::transport_layer::TransportLayer;

const PEER: [u8; 2] = [0x11, 0xFA];
const GA_A: [u8; 2] = [0x09, 0x62];
const GA_B: [u8; 2] = [0x09, 0x63];
const GROUP_ADDRESSES: GroupAddressTable = GroupAddressTable::new(&[
    GroupAddress::from_parts(1, 1, 98),
    GroupAddress::from_parts(1, 1, 99),
]);

const SWITCH: Asap = Asap::new(1);
const STATUS: Asap = Asap::new(2);

const READ: [u8; 1] = [0x00];
const RESPONSE_ON: [u8; 1] = [0x41];
const WRITE_ON: [u8; 1] = [0x81];

fn b1(flags: &[ConfigFlag]) -> Descriptor {
    Descriptor::new(ValueType::bits(1), Priority::Low, flags)
}

/// A group telegram from PEER.
fn received(ga: [u8; 2], apdu: &[u8]) -> Vec<u8> {
    let header = [0xBC, PEER[0], PEER[1], ga[0], ga[1], 0xE0 | apdu.len() as u8, 0x00];
    [&header[..], apdu].concat()
}

/// A group telegram from us, as the simulator returns it.
fn sent(ga: [u8; 2], apdu: &[u8]) -> Vec<u8> {
    let header = [0xBC, ME[0], ME[1], ga[0], ga[1], 0xF0 | apdu.len() as u8, 0x00];
    with_checksum(&[&header[..], apdu].concat())
}

#[derive(Clone, Copy)]
struct Device {
    sim: &'static Ncn5130Sim,
    gos: &'static GroupObjects,
}

impl Device {
    fn run<F: std::future::Future>(
        descriptors: &[Descriptor],
        associations: &[(u16, u16)],
        body: impl FnOnce(Device) -> F,
    ) -> F::Output {
        init_pool();
        let descriptors = leak_slice(descriptors);
        let associations: Vec<_> = associations
            .iter()
            .map(|&(tsap, asap)| Association::new(tsap, asap))
            .collect();
        let values = Box::leak(vec![0; value_bytes(descriptors)].into_boxed_slice());
        let ram = Box::leak(vec![GroupObjectRam::new(); descriptors.len()].into_boxed_slice());
        let device = Device {
            sim: leak(Ncn5130Sim::new()),
            gos: leak(GroupObjects::new(descriptors, values, ram)),
        };
        let bus = leak(Bus::new());
        let driver = NCN51Driver::new(device.sim.uart(), NoLed, bus);
        let network = NetworkLayer::new(DataLinkLayer::new(bus));
        let transport = TransportLayer::new(network, GROUP_ADDRESSES);
        let associations = AssociationTable::new(leak_slice(&associations));
        let application = ApplicationLayer::new(transport, associations, device.gos);
        run(with_background(
            body(device),
            join(driver.run(), application.run()),
        ))
    }

    async fn receive(&self, telegram: &[u8]) {
        self.sim.bus_receive(telegram).await;
        self.sim.expect_ack().await;
    }

    fn no_update(&self) -> bool {
        poll_once(self.gos.updated()).is_pending()
    }
}

#[test]
fn group_value_read_is_answered_from_the_stored_value() {
    Device::run(&[b1(&[Communication, Read])], &[(1, 1)], |d| async move {
        d.gos.store(SWITCH, &B1::new(true));
        d.receive(&received(GA_A, &READ)).await;
        assert_eq!(d.sim.transmit(true).await, sent(GA_A, &RESPONSE_ON));
    });
}

#[test]
fn group_value_read_needs_c_and_r() {
    for flags in [&[Communication][..], &[Read]] {
        Device::run(&[b1(flags)], &[(1, 1)], |d| async move {
            d.receive(&received(GA_A, &READ)).await;
            settle().await;
            assert_eq!(d.sim.host_bytes_now(), [], "{flags:?}");
        });
    }
}

#[test]
fn group_value_write_updates_the_object() {
    Device::run(&[b1(&[Communication, Write])], &[(1, 1)], |d| async move {
        d.receive(&received(GA_A, &WRITE_ON)).await;
        assert_eq!(d.gos.updated().await, SWITCH);
        assert!(d.gos.get::<B1>(SWITCH).value());
        assert!(d.no_update());
    });
}

#[test]
fn group_value_write_needs_w() {
    Device::run(&[b1(&[Communication, Read])], &[(1, 1)], |d| async move {
        d.receive(&received(GA_A, &WRITE_ON)).await;
        settle().await;
        assert!(d.no_update());
        assert!(!d.gos.get::<B1>(SWITCH).value());
    });
}

#[test]
fn group_value_response_needs_u() {
    Device::run(&[b1(&[Communication, Write])], &[(1, 1)], |d| async move {
        d.receive(&received(GA_A, &RESPONSE_ON)).await;
        settle().await;
        assert!(d.no_update());
    });
    Device::run(&[b1(&[Communication, Update])], &[(1, 1)], |d| async move {
        d.receive(&received(GA_A, &RESPONSE_ON)).await;
        assert_eq!(d.gos.updated().await, SWITCH);
        assert!(d.gos.get::<B1>(SWITCH).value());
    });
}

#[test]
fn group_frame_to_an_unknown_address_is_ignored() {
    Device::run(&[b1(&[Communication, Write])], &[(1, 1)], |d| async move {
        d.receive(&received([0x09, 0x64], &WRITE_ON)).await;
        settle().await;
        assert!(d.no_update());
    });
}

#[test]
fn set_sends_a_write_on_the_sending_address() {
    let flags = [Communication, Transmit];
    Device::run(&[b1(&flags)], &[(2, 1), (1, 1)], |d| async move {
        d.gos.set(SWITCH, &B1::new(true));
        assert_eq!(d.sim.transmit(true).await, sent(GA_B, &WRITE_ON));
        settle().await;
        assert_eq!(d.gos.status(SWITCH), TransmissionStatus::IdleOk);

        d.gos.set(SWITCH, &B1::new(false));
        assert_eq!(d.sim.transmit(false).await, sent(GA_B, &[0x80]));
        settle().await;
        assert_eq!(d.gos.status(SWITCH), TransmissionStatus::IdleError);
    });
}

#[test]
fn request_read_is_answered_through_an_update() {
    let flags = [Communication, Transmit, Update];
    Device::run(&[b1(&flags)], &[(1, 1)], |d| async move {
        d.gos.request_read(SWITCH);
        assert_eq!(d.sim.transmit(true).await, sent(GA_A, &READ));
        d.receive(&received(GA_A, &RESPONSE_ON)).await;
        assert_eq!(d.gos.updated().await, SWITCH);
        assert!(d.gos.get::<B1>(SWITCH).value());
        assert_eq!(d.gos.status(SWITCH), TransmissionStatus::IdleOk);
    });
}

#[test]
fn objects_with_i_are_read_at_startup() {
    let flags = [Communication, Transmit, ReadOnInit];
    Device::run(&[b1(&flags)], &[(1, 1)], |d| async move {
        assert_eq!(d.sim.transmit(true).await, sent(GA_A, &READ));
    });
}

#[test]
fn one_write_updates_every_associated_object() {
    let flags = [Communication, Write];
    Device::run(&[b1(&flags), b1(&flags)], &[(1, 1), (1, 2)], |d| async move {
        d.receive(&received(GA_A, &WRITE_ON)).await;
        assert_eq!(d.gos.updated().await, SWITCH);
        assert_eq!(d.gos.updated().await, STATUS);
        assert!(d.gos.get::<B1>(STATUS).value());
    });
}

#[test]
fn own_write_updates_other_objects_on_the_same_address() {
    let objects = [b1(&[Communication, Transmit, Write]), b1(&[Communication, Write])];
    Device::run(&objects, &[(1, 1), (1, 2)], |d| async move {
        d.gos.set(SWITCH, &B1::new(true));
        assert_eq!(d.sim.transmit(true).await, sent(GA_A, &WRITE_ON));
        assert_eq!(d.gos.updated().await, STATUS);
        assert!(d.gos.get::<B1>(STATUS).value());
        assert!(d.no_update());
    });
}

#[test]
fn data_ack_survives_a_transmit_request() {
    Device::run(&[b1(&[Communication, Transmit])], &[(1, 1)], |d| async move {
        d.receive(&[0xB0, PEER[0], PEER[1], ME[0], ME[1], 0x60, 0x80])
            .await;
        d.receive(&[0xB0, PEER[0], PEER[1], ME[0], ME[1], 0x61, 0x43, 0x00])
            .await;

        let t_ack = d.sim.host_request().await;
        d.gos.set(SWITCH, &B1::new(true));
        settle().await;
        d.sim.echo_and_confirm(&t_ack, true);
        assert_eq!(d.sim.transmit(true).await, sent(GA_A, &WRITE_ON));

        d.receive(&[0xB0, PEER[0], PEER[1], ME[0], ME[1], 0x61, 0x47, 0x00])
            .await;
        let expected = [0xB0, ME[0], ME[1], PEER[0], PEER[1], 0x70, 0xC6];
        assert_eq!(d.sim.transmit(true).await, with_checksum(&expected));
    });
}

#[test]
fn connection_oriented_data_is_acked() {
    Device::run(&[], &[], |d| async move {
        d.receive(&[0xB0, PEER[0], PEER[1], ME[0], ME[1], 0x60, 0x80])
            .await;
        d.receive(&[0xB0, PEER[0], PEER[1], ME[0], ME[1], 0x61, 0x43, 0x00])
            .await;

        let expected = [0xB0, ME[0], ME[1], PEER[0], PEER[1], 0x70, 0xC2];
        assert_eq!(d.sim.transmit(true).await, with_checksum(&expected));
    });
}
