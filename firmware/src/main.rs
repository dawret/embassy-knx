#![no_std]
#![no_main]

use assign_resources::assign_resources;
use defmt::*;
use embassy_executor::Spawner;
use embassy_knx::application_layer::{self, ApplicationLayer, SERVICE_CHANNEL_SIZE};
use embassy_knx::bus::Bus;
use embassy_knx::data_point::*;
use embassy_knx::frame::{self, LargeFrameBlock, SmallFrameBlock};
use embassy_knx::ncn51_driver::NCN51Driver;
use embassy_knx::{data_link_layer, network_layer, transport_layer};
use embassy_nrf::buffered_uarte::{self, BufferedUarte};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_nrf::{bind_interrupts, peripherals, uarte, Peri};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use static_cell::StaticCell;

use {defmt_rtt as _, panic_probe as _};

assign_resources! {
    uart: UartResources {
        serial: SERIAL0,
        // TIMER0/1 are left for a radio stack.
        timer: TIMER2,
        ppi_ch1: PPI_CH0,
        ppi_ch2: PPI_CH1,
        ppi_group: PPI_GROUP0,
        rx: P1_10,
        tx: P1_11,
        led: P1_07,
    }
}

bind_interrupts!(struct Irqs {
    SERIAL0 => buffered_uarte::InterruptHandler<peripherals::SERIAL0>;
});

const SMALL_FRAME_POOL_SIZE: usize = 12;
const LARGE_FRAME_POOL_SIZE: usize = 2;

static BUS: Bus = Bus::new();

type Driver = NCN51Driver<BufferedUarte<'static>, Output<'static>>;

/// Must be even.
const UART_RX_BUF_SIZE: usize = 256;
const UART_TX_BUF_SIZE: usize = 64;

fn new_driver(resources: UartResources) -> Driver {
    static RX_BUF: StaticCell<[u8; UART_RX_BUF_SIZE]> = StaticCell::new();
    static TX_BUF: StaticCell<[u8; UART_TX_BUF_SIZE]> = StaticCell::new();
    let led_rx = Output::new(resources.led, Level::Low, OutputDrive::Standard);
    let mut config = uarte::Config::default();
    config.parity = uarte::Parity::Included;
    config.baudrate = uarte::Baudrate::Baud38400;
    let uart = BufferedUarte::new(
        resources.serial,
        resources.timer,
        resources.ppi_ch1,
        resources.ppi_ch2,
        resources.ppi_group,
        resources.rx,
        resources.tx,
        Irqs,
        config,
        RX_BUF.init([0; UART_RX_BUF_SIZE]),
        TX_BUF.init([0; UART_TX_BUF_SIZE]),
    );
    NCN51Driver::new(uart, led_rx, &BUS)
}

#[embassy_executor::task]
async fn uart_task(driver: Driver) -> ! {
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
        CriticalSectionRawMutex,
        application_layer::ApplicationServiceInd,
        SERVICE_CHANNEL_SIZE,
    > = Channel::new();
    static SERVICE_CHANNEL_TX: Channel<
        CriticalSectionRawMutex,
        application_layer::ApplicationServiceRes,
        SERVICE_CHANNEL_SIZE,
    > = Channel::new();
    static SMALL_BLOCKS: StaticCell<[SmallFrameBlock; SMALL_FRAME_POOL_SIZE]> = StaticCell::new();
    static LARGE_BLOCKS: StaticCell<[LargeFrameBlock; LARGE_FRAME_POOL_SIZE]> = StaticCell::new();
    frame::init_frame_pool(
        SMALL_BLOCKS.init([const { SmallFrameBlock::new() }; SMALL_FRAME_POOL_SIZE]),
        LARGE_BLOCKS.init([const { LargeFrameBlock::new() }; LARGE_FRAME_POOL_SIZE]),
    );

    info!("Hello from rust! We're on core: {}", t);
    info!("My address: {}", embassy_knx::settings::ADDRESS);

    let driver = new_driver(r.uart);
    let data_link = data_link_layer::DataLinkLayer::new(&BUS);
    let network = network_layer::NetworkLayer::new(data_link);
    let transport = transport_layer::TransportLayer::new(network);
    let application = application_layer::ApplicationLayer::new(
        transport,
        SERVICE_CHANNEL_TX.receiver(),
        SERVICE_CHANNEL_RX.sender(),
    );

    spawner.spawn(unwrap!(uart_task(driver)));
    spawner.spawn(unwrap!(application_task(application)));

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
