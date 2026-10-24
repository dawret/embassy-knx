#![no_std]
#![no_main]

use assign_resources::assign_resources;
use defmt::*;
use embassy_executor::Spawner;
use embassy_knx::application_layer::ApplicationLayer;
use embassy_knx::bus::Bus;
use embassy_knx::data_point::B1;
use embassy_knx::frame::{self, GroupAddress, LargeFrameBlock, Priority, SmallFrameBlock};
use embassy_knx::group_address_table::GroupAddressTable;
use embassy_knx::group_object::ConfigFlag::*;
use embassy_knx::group_object::{
    value_bytes, Asap, Descriptor, GroupObjectRam, GroupObjects, ValueType,
};
use embassy_knx::group_object_association_table::{Association, AssociationTable};
use embassy_knx::ncn51_driver::NCN51Driver;
use embassy_knx::{data_link_layer, network_layer, transport_layer};
use embassy_nrf::buffered_uarte::{self, BufferedUarte};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_nrf::{bind_interrupts, peripherals, uarte, Peri};
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

const SWITCH: Asap = Asap::new(1);
const STATUS: Asap = Asap::new(2);

const GROUP_ADDRESSES: GroupAddressTable = GroupAddressTable::new(&[
    GroupAddress::from_parts(1, 1, 98),
    GroupAddress::from_parts(1, 1, 99),
]);
const ASSOCIATIONS: AssociationTable =
    AssociationTable::new(&[Association::new(1, 1), Association::new(2, 2)]);
static DESCRIPTORS: [Descriptor; 2] = [
    Descriptor::new(ValueType::bits(1), Priority::Low, &[Communication, Write]),
    Descriptor::new(
        ValueType::bits(1),
        Priority::Low,
        &[Communication, Read, Transmit],
    ),
];

fn new_group_objects() -> &'static GroupObjects {
    static VALUES: StaticCell<[u8; value_bytes(&DESCRIPTORS)]> = StaticCell::new();
    static RAM: StaticCell<[GroupObjectRam; DESCRIPTORS.len()]> = StaticCell::new();
    static GROUP_OBJECTS: StaticCell<GroupObjects> = StaticCell::new();
    GROUP_OBJECTS.init(GroupObjects::new(
        &DESCRIPTORS,
        VALUES.init([0; value_bytes(&DESCRIPTORS)]),
        RAM.init([const { GroupObjectRam::new() }; DESCRIPTORS.len()]),
    ))
}

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
    let transport = transport_layer::TransportLayer::new(network, GROUP_ADDRESSES);
    let group_objects = new_group_objects();
    let application = ApplicationLayer::new(transport, ASSOCIATIONS, group_objects);

    spawner.spawn(unwrap!(uart_task(driver)));
    spawner.spawn(unwrap!(application_task(application)));

    loop {
        if group_objects.updated().await == SWITCH {
            let on: B1 = group_objects.get(SWITCH);
            info!("Switch: {}", on.value());
            led.set_level(Level::from(on.value()));
            group_objects.set(STATUS, &on);
        }
    }
}
