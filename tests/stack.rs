mod common;

use common::*;
use embassy_futures::join::join;
use embassy_knx::application_layer::{
    ApplicationLayer, ApplicationServiceInd, ApplicationServiceRes, GroupReadResponse,
    SERVICE_CHANNEL_SIZE,
};
use embassy_knx::bus::Bus;
use embassy_knx::data_link_layer::DataLinkLayer;
use embassy_knx::data_point::{DataPoint, B1};
use embassy_knx::frame::Priority;
use embassy_knx::ncn51_driver::NCN51Driver;
use embassy_knx::network_layer::NetworkLayer;
use embassy_knx::transport_layer::TransportLayer;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

type IndChannel = Channel<CriticalSectionRawMutex, ApplicationServiceInd, SERVICE_CHANNEL_SIZE>;
type ResChannel = Channel<CriticalSectionRawMutex, ApplicationServiceRes, SERVICE_CHANNEL_SIZE>;

const PEER: [u8; 2] = [0x11, 0xFA];

#[derive(Clone, Copy)]
struct Device {
    sim: &'static Ncn5130Sim,
    ind: &'static IndChannel,
    res: &'static ResChannel,
}

impl Device {
    fn run<F: std::future::Future>(body: impl FnOnce(Device) -> F) -> F::Output {
        init_pool();
        let device = Device {
            sim: leak(Ncn5130Sim::new()),
            ind: leak(Channel::new()),
            res: leak(Channel::new()),
        };
        let bus = leak(Bus::new());
        let driver = NCN51Driver::new(device.sim.uart(), NoLed, bus);
        let transport = TransportLayer::new(NetworkLayer::new(DataLinkLayer::new(bus)));
        let application =
            ApplicationLayer::new(transport, device.res.receiver(), device.ind.sender());
        run(with_background(
            body(device),
            join(driver.run(), application.run()),
        ))
    }
}

#[test]
fn group_value_read_is_answered() {
    Device::run(|d| async move {
        d.sim
            .bus_receive(&[0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE1, 0x00, 0x00])
            .await;
        d.sim.expect_ack().await;

        assert_eq!(d.ind.receive().await, ApplicationServiceInd::GroupValueRead(0));
        let value = DataPoint::B1(B1::new(true));
        let response = GroupReadResponse::new(0, value, Priority::Normal);
        d.res.send(ApplicationServiceRes::GroupValueRead(response)).await;

        let expected = [0xB4, ME[0], ME[1], 0x09, 0x62, 0xF1, 0x00, 0x41];
        assert_eq!(d.sim.transmit(true).await, with_checksum(&expected));
    });
}

#[test]
fn connection_oriented_data_is_acked() {
    Device::run(|d| async move {
        d.sim
            .bus_receive(&[0xB0, PEER[0], PEER[1], ME[0], ME[1], 0x60, 0x80])
            .await;
        d.sim.expect_ack().await;
        d.sim
            .bus_receive(&[0xB0, PEER[0], PEER[1], ME[0], ME[1], 0x61, 0x43, 0x00])
            .await;
        d.sim.expect_ack().await;

        let expected = [0xB0, ME[0], ME[1], PEER[0], PEER[1], 0x70, 0xC2];
        assert_eq!(d.sim.transmit(true).await, with_checksum(&expected));
    });
}
