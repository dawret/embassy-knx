use crate::frame::*;

use crate::UartResources;
use core::panic;
use defmt::*;
use embassy_futures::select::{select, Either};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_nrf::peripherals::SERIAL0;
use embassy_nrf::{bind_interrupts, uarte};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, WithTimeout};
use heapless::Vec;
use heapless::{box_pool, pool::boxed::BoxBlock};
use static_cell::StaticCell;

type Serial = uarte::Uarte<'static, SERIAL0>;

//TODO: Add "bytes_read" information
#[derive(Format)]
enum TransferError {
    TimeoutError(usize),
    ReadError(uarte::Error),
    FrameError(FrameError),
    InvalidData(u8),
}

impl From<uarte::Error> for TransferError {
    fn from(err: uarte::Error) -> Self {
        TransferError::ReadError(err)
    }
}

impl From<FrameError> for TransferError {
    fn from(err: FrameError) -> Self {
        TransferError::FrameError(err)
    }
}

/*struct Reset {}

impl Reset {}
*/
#[allow(dead_code)]
pub enum CtrlMsg {
    /*Reset,
    State,
    Configure,
    SystemStat,
    StopMode,
    IntRegRd,*/
}

#[allow(dead_code)]
mod commands {
    pub const U_RESET_REQ: u8 = 0x01;
    pub const U_STATE_REQ: u8 = 0x02;
    pub const U_SET_BUSY_REQ: u8 = 0x03;
    pub const U_QUIT_BUSY_REQ: u8 = 0x04;
    pub const U_BUSMON_REQ: u8 = 0x05;
    pub const U_SET_ADDRESS_REQ: u8 = 0xF1;
    pub const U_SET_REPETITION_REQ: u8 = 0xF2;
    pub const U_L_DATA_OFFSET_REQ: u8 = 0x08;
    pub const U_SYSTEM_STATE_REQ: u8 = 0x0D;
    pub const U_STOP_MODE_REQ: u8 = 0x0E;
    pub const U_EXIT_STOP_MODE_REQ: u8 = 0x0F;
    pub const U_ACKN_REQ: u8 = 0x10;
    pub const U_CONFIGURE_REQ: u8 = 0x18;
    pub const U_INT_REG_WR_REQ: u8 = 0x28;
    pub const U_INT_REG_RD_REQ: u8 = 0x38;
    pub const U_POLLING_STATE_REQ: u8 = 0xE0;
    pub const U_L_DATA_START_REQ: u8 = 0x80;
    pub const U_L_DATA_CONT_REQ: u8 = 0x80;
    pub const U_L_DATA_END_REQ: u8 = 0x40;
}

#[derive(Format)]
pub enum ConStatus {
    Ok,
    NotOk,
}

#[allow(dead_code)]
enum AckTypes {
    NACK = 0x4,
    BUSY = 0x2,
    ACK = 0x1,
}

bind_interrupts!(struct Irqs {
    SERIAL0 => uarte::InterruptHandler<SERIAL0>;
});

const MAX_FRAME_SIZE: usize = 264;
const CHANNEL_SIZE: usize = 8;
box_pool!(FRAME_POOL: Vec<u8, MAX_FRAME_SIZE>);
//box_pool!(FRAME_POOL: dyn FrameWriter);
pub static CTRL_CHANNEL: Channel<ThreadModeRawMutex, CtrlMsg, CHANNEL_SIZE> = Channel::new();
pub static CON_SIGNAL: Signal<ThreadModeRawMutex, ConStatus> = Signal::new();
pub static FRAME_CHANNEL_TX: Channel<ThreadModeRawMutex, Frame, CHANNEL_SIZE> = Channel::new();
pub static FRAME_CHANNEL_RX: Channel<ThreadModeRawMutex, Frame, CHANNEL_SIZE> = Channel::new();

pub struct NCN51Driver {
    uarte: Serial,
    led: Output<'static>,
}

impl NCN51Driver {
    const BUS_SILENCE_US: u64 = 2600;
    const FRAME_TYPE_MASK: u8 = 0xd3;
    const FRAME_TYPE_STD: u8 = 0x90;
    const FRAME_TYPE_EXT: u8 = 0x10;

    pub fn new(resources: UartResources) -> Self {
        let led_rx = Output::new(resources.led, Level::Low, OutputDrive::Standard);
        let mut config = uarte::Config::default();
        config.parity = uarte::Parity::INCLUDED;
        config.baudrate = uarte::Baudrate::BAUD38400;
        let uart = uarte::Uarte::new(resources.serial, Irqs, resources.rx, resources.tx, config);

        static FRAME_BUFFER: StaticCell<[BoxBlock<Vec<u8, MAX_FRAME_SIZE>>; CHANNEL_SIZE]> =
            StaticCell::new();
        let blocks = FRAME_BUFFER.init([const { BoxBlock::new() }; CHANNEL_SIZE]);
        for block in blocks {
            FRAME_POOL.manage(block);
        }

        Self {
            uarte: uart,
            led: led_rx,
        }
    }

    fn is_frame_start(&self, ctrl: u8) -> bool {
        [Self::FRAME_TYPE_STD, Self::FRAME_TYPE_EXT].contains(&(ctrl & Self::FRAME_TYPE_MASK))
    }

    async fn ack(&mut self, ack: AckTypes) -> Result<(), TransferError> {
        self.uarte
            .write(&[commands::U_ACKN_REQ | ack as u8])
            .await?;
        Ok(())
    }

    pub async fn run(mut self) -> ! {
        loop {
            let mut buf = [0; 1];
            match select(
                self.uarte.read(&mut buf),
                FRAME_CHANNEL_RX.ready_to_receive(),
            )
            .await
            {
                Either::First(ret) => {
                    if let Err(e) = ret {
                        info!("Reception error: {}", e);
                    } else if self.is_frame_start(buf[0]) {
                        match self.receive_frame(buf[0]).await {
                            Ok(frame) => {
                                self.led.toggle();
                                FRAME_CHANNEL_TX.send(frame).await;
                            }
                            Err(e) => {
                                info!("Reception error: {}", e);
                                self.read_with_timeout_ignore_errors().await;
                                //TODO: Handle NACK

                                /*if let Err(e) = self.ack(AckTypes::NACK).await {
                                    info!("Failed to send NACK: {}", e);
                                }*/
                            }
                        }
                    } else {
                        info!("Unexpected byte: {:x}", buf[0]);
                        // unexpected byte
                    }
                }
                Either::Second(_) => {
                    let frame = FRAME_CHANNEL_RX.receive().await;
                    match self.send_frame(frame).await {
                        Ok(con_status) => {
                            CON_SIGNAL.signal(con_status);
                        }
                        Err(e) => {
                            error!("Transmission error: {}", e);
                            CON_SIGNAL.signal(ConStatus::NotOk);
                        }
                    }
                }
            }
        }
    }

    async fn send_frame(&mut self, mut frame: Frame) -> Result<ConStatus, TransferError> {
        let buf = frame.data();
        let checksum = frame.checksum();
        let mut cmd = [0; 1];
        cmd[0] = commands::U_L_DATA_START_REQ;
        self.uarte.write_from_ram(&mut cmd).await?;
        self.uarte.write_from_ram(&buf[0..1]).await?;
        for i in 1..buf.len() - 1 {
            cmd[0] = commands::U_L_DATA_CONT_REQ + i as u8;
            self.uarte.write_from_ram(&mut cmd).await?;
            self.uarte.write_from_ram(&buf[i..i + 1]).await?;
        }
        cmd[0] = commands::U_L_DATA_END_REQ | (buf.len() as u8 - 1);
        self.uarte.write_from_ram(&mut cmd).await?;
        cmd[0] = checksum;
        self.uarte.write_from_ram(&mut cmd).await?;
        let rx_buf = frame.mut_data();
        for i in 0..rx_buf.len() {
            self.read_with_timeout(&mut rx_buf[i..i + 1]).await?;
        }
        self.uarte.read(&mut cmd).await?;
        if cmd[0] & 0x7f != 0xb {
            error!("Invalid L_Data.con: {:x}", cmd[0]);
            return Err(TransferError::InvalidData(cmd[0]));
        }
        let con_status = if (cmd[0] >> 7) != 0 {
            ConStatus::Ok
        } else {
            ConStatus::NotOk
        };
        //info!("Frame transfer complete. L_Data.con: {:x}", cmd[0]);
        Ok(con_status)
    }

    async fn receive_frame_int<T: FrameWriter>(&mut self, ctrl: u8) -> Result<T, TransferError> {
        let mut frame = T::new(T::MAX_FRAME_SIZE)?;
        frame.mut_data()[0] = ctrl;
        let bytes_read = 1 + self
            .read_with_timeout(&mut frame.mut_data()[1..T::HEADER_LENGTH])
            .await?;
        self.ack_if_addressed(&frame.dst_addr()).await?;
        let bytes_read = bytes_read
            + match self
                .read_with_timeout(&mut frame.mut_data()[T::HEADER_LENGTH..])
                .await
            {
                Ok(bytes) => Ok(bytes),
                Err(TransferError::TimeoutError(bytes)) => Ok(bytes),
                Err(e) => Err(e),
            }?;
        if bytes_read < T::MIN_FRAME_SIZE
            || bytes_read > T::MAX_FRAME_SIZE
            || bytes_read != frame.length() as usize
        {
            return Err(TransferError::FrameError(FrameError::InvalidLength));
        }
        if frame.data()[bytes_read] != frame.checksum() {
            return Err(TransferError::FrameError(FrameError::Checksum));
        }
        frame.set_length(bytes_read)?;

        Ok(frame)
    }

    async fn receive_frame(&mut self, ctrl: u8) -> Result<Frame, TransferError> {
        match ctrl & Self::FRAME_TYPE_MASK {
            Self::FRAME_TYPE_STD => {
                let frame = self.receive_frame_int(ctrl).await?;
                Ok(Frame::Standard(frame))
            }
            Self::FRAME_TYPE_EXT => {
                let frame = self.receive_frame_int(ctrl).await?;
                Ok(Frame::Extended(frame))
            }
            _ => panic!(),
        }
    }

    async fn ack_if_addressed(&mut self, dst_addr: &Address) -> Result<(), TransferError> {
        if match dst_addr {
            Address::Individual(ref addr) => addr == &crate::settings::ADDRESS,
            Address::Group(_) => true,
            _ => false,
        } {
            self.ack(AckTypes::ACK).await
        } else {
            Ok(())
        }
    }

    async fn read_with_timeout(&mut self, buf: &mut [u8]) -> Result<usize, TransferError> {
        let mut read = 0;
        for i in 0..buf.len() {
            self.uarte
                .read(&mut buf[i..i + 1])
                .with_timeout(Duration::from_micros(Self::BUS_SILENCE_US))
                .await
                .map_err(|_| TransferError::TimeoutError(read))??;
            read = read + 1;
        }
        Ok(read)
    }

    async fn read_with_timeout_ignore_errors(&mut self) -> usize {
        let mut buf = [0; 1];
        let mut read = 0;
        warn!("Dumping data until timeout ignoreing errors...");
        loop {
            match self
                .uarte
                .read(&mut buf)
                .with_timeout(Duration::from_micros(Self::BUS_SILENCE_US))
                .await
            {
                Ok(_) => {
                    read = read + 1;
                }
                Err(_) => return read,
            }
        }
    }
}
