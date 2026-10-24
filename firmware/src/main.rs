#![no_std]
#![no_main]

use assign_resources::assign_resources;
use defmt::*;
use embassy_executor::Spawner;
use embassy_knx::application_layer::{self, ApplicationLayer, SERVICE_CHANNEL_SIZE};
use embassy_knx::bus::Bus;
use embassy_knx::data_point::*;
use embassy_knx::frame::{self, FrameBlock};
use embassy_knx::ncn51_driver::NCN51Driver;
use embassy_knx::{data_link_layer, network_layer, transport_layer};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_nrf::{bind_interrupts, peripherals, uarte, Peri};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use static_cell::StaticCell;

use {defmt_rtt as _, panic_probe as _};

assign_resources! {
    uart: UartResources {
        serial: SERIAL0,
        rx: P1_10,
        tx: P1_11,
        led: P1_07,
    }
}

bind_interrupts!(struct Irqs {
    SERIAL0 => uarte::InterruptHandler<peripherals::SERIAL0>;
});

const FRAME_POOL_SIZE: usize = 8;

static BUS: Bus = Bus::new();

/// embassy-nrf's `Uarte` implements `embedded_io_async::Write` but not `Read`.
struct UarteIo(uarte::Uarte<'static>);

impl embedded_io_async::ErrorType for UarteIo {
    type Error = uarte::Error;
}

impl embedded_io_async::Read for UarteIo {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await?;
        Ok(buf.len())
    }
}

impl embedded_io_async::Write for UarteIo {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await?;
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        // `Uarte::write` only returns once the transfer has completed.
        Ok(())
    }
}

type Driver = NCN51Driver<UarteIo, Output<'static>>;

fn new_driver(resources: UartResources) -> Driver {
    let led_rx = Output::new(resources.led, Level::Low, OutputDrive::Standard);
    let mut config = uarte::Config::default();
    config.parity = uarte::Parity::Included;
    config.baudrate = uarte::Baudrate::Baud38400;
    let uart = uarte::Uarte::new(resources.serial, resources.rx, resources.tx, Irqs, config);
    NCN51Driver::new(UarteIo(uart), led_rx, &BUS)
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
    static FRAME_BLOCKS: StaticCell<[FrameBlock; FRAME_POOL_SIZE]> = StaticCell::new();
    frame::init_frame_pool(FRAME_BLOCKS.init([const { FrameBlock::new() }; FRAME_POOL_SIZE]));

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
