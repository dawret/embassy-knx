#![no_std]
#![no_main]

mod application_layer;
mod data_link_layer;
mod data_point;
mod frame;
mod group_object;
mod ncn51_driver;
mod network_layer;
mod settings;
mod transport_layer;

use application_layer::ApplicationLayer;
use assign_resources::assign_resources;
use data_point::*;
use defmt::*;
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_nrf::peripherals;
use embassy_sync::channel::Channel;
use ncn51_driver::NCN51Driver;

use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;

use {defmt_rtt as _, panic_probe as _};

assign_resources! {
    uart: UartResources {
        serial: SERIAL0,
        rx: P1_10,
        tx: P1_11,
        led: P1_07,
    }
}

#[embassy_executor::task]
async fn uart_task(driver: NCN51Driver) -> ! {
    driver.run().await;
}

#[embassy_executor::task]
async fn application_task(application: ApplicationLayer) -> ! {
    application.run().await;
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());
    let t = embassy_nrf::pac::DCNF.cpuid().read().cpuid();
    let mut led = Output::new(p.P1_05, Level::Low, OutputDrive::Standard);
    let r = split_resources!(p);
    static SERVICE_CHANNEL_RX: Channel<
        ThreadModeRawMutex,
        application_layer::ApplicationServiceInd,
        4,
    > = Channel::new();
    static SERVICE_CHANNEL_TX: Channel<
        ThreadModeRawMutex,
        application_layer::ApplicationServiceRes,
        4,
    > = Channel::new();

    info!("Hello from rust! We're on core: {}", t);
    info!("My address: {}", crate::settings::ADDRESS);

    let driver = ncn51_driver::NCN51Driver::new(r.uart);
    let data_link = data_link_layer::DataLinkLayer::new(
        ncn51_driver::FRAME_CHANNEL_TX.receiver(),
        ncn51_driver::FRAME_CHANNEL_RX.sender(),
    );
    let network = network_layer::NetworkLayer::new(data_link);
    let transport = transport_layer::TransportLayer::new(network);
    let application = application_layer::ApplicationLayer::new(
        transport,
        SERVICE_CHANNEL_TX.receiver(),
        SERVICE_CHANNEL_RX.sender(),
    );

    spawner.spawn(uart_task(driver)).unwrap();
    spawner.spawn(application_task(application)).unwrap();

    loop {
        let ind = SERVICE_CHANNEL_RX.receive().await;
        match ind {
            application_layer::ApplicationServiceInd::GroupValueRead(asap) => {
                info!("Read request on ASAP: {}", asap);
                let resp = application_layer::GroupReadResponse::new(
                    0,
                    DataPoint::B1(B1::new(true)),
                    frame::Priority::Normal,
                );
                SERVICE_CHANNEL_TX
                    .send(application_layer::ApplicationServiceRes::GroupValueRead(
                        resp,
                    ))
                    .await;
            }
        }

        led.toggle();
    }
}
