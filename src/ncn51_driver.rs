use crate::bus::{Bus, ConStatus};
use crate::frame::*;

use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, WithTimeout};
use embedded_hal::digital::StatefulOutputPin;
use embedded_io_async::{ErrorKind, Read, Write};

//TODO: Add "bytes_read" information
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
// The payloads are only for logging, and `Debug` doesn't count as a read.
#[cfg_attr(not(feature = "defmt"), allow(dead_code))]
enum TransferError {
    TimeoutError(usize),
    ReadError(ErrorKind),
    FrameError(FrameError),
    InvalidData(u8),
}

fn io_error<E: embedded_io_async::Error>(err: E) -> TransferError {
    TransferError::ReadError(err.kind())
}

impl From<FrameError> for TransferError {
    fn from(err: FrameError) -> Self {
        TransferError::FrameError(err)
    }
}

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

#[allow(dead_code)]
enum AckTypes {
    NACK = 0x4,
    BUSY = 0x2,
    ACK = 0x1,
}

const CHANNEL_SIZE: usize = 8;
const TX_CHUNK: usize = 64;
pub static CTRL_CHANNEL: Channel<CriticalSectionRawMutex, CtrlMsg, CHANNEL_SIZE> = Channel::new();

pub struct NCN51Driver<U, L> {
    uart: U,
    led: L,
    bus: &'static Bus,
}

impl<U: Read + Write, L: StatefulOutputPin> NCN51Driver<U, L> {
    const BUS_SILENCE_US: u64 = 2600;
    const FRAME_TYPE_MASK: u8 = 0xd3;
    const FRAME_TYPE_STD: u8 = 0x90;
    const FRAME_TYPE_EXT: u8 = 0x10;

    pub fn new(uart: U, led: L, bus: &'static Bus) -> Self {
        Self { uart, led, bus }
    }

    fn is_frame_start(&self, ctrl: u8) -> bool {
        [Self::FRAME_TYPE_STD, Self::FRAME_TYPE_EXT].contains(&(ctrl & Self::FRAME_TYPE_MASK))
    }

    async fn ack(&mut self, ack: AckTypes) -> Result<(), TransferError> {
        self.write_byte(commands::U_ACKN_REQ | ack as u8).await
    }

    async fn write_byte(&mut self, byte: u8) -> Result<(), TransferError> {
        let buf = [byte];
        self.uart.write_all(&buf).await.map_err(io_error)
    }

    async fn read_byte(&mut self, buf: &mut [u8]) -> Result<(), TransferError> {
        match self.uart.read(&mut buf[..1]).await.map_err(io_error)? {
            0 => Err(TransferError::ReadError(ErrorKind::BrokenPipe)),
            _ => Ok(()),
        }
    }

    pub async fn run(mut self) -> ! {
        loop {
            let mut buf = [0; 1];
            let bus = self.bus;
            match select(self.read_byte(&mut buf), bus.bus_tx.ready_to_receive()).await
            {
                Either::First(ret) => {
                    if let Err(e) = ret {
                        info!("Reception error: {}", e);
                    } else if self.is_frame_start(buf[0]) {
                        match self.receive_frame(buf[0]).await {
                            Ok(frame) => {
                                let _ = self.led.toggle();
                                self.bus.bus_rx.send(frame).await;
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
                    }
                }
                Either::Second(_) => {
                    let frame = self.bus.bus_tx.receive().await;
                    match self.send_frame(frame).await {
                        Ok(con_status) => {
                            self.bus.con.signal(con_status);
                        }
                        Err(e) => {
                            error!("Transmission error: {}", e);
                            self.bus.con.signal(ConStatus::NotOk);
                        }
                    }
                }
            }
        }
    }

    async fn send_frame(&mut self, mut frame: Frame) -> Result<ConStatus, TransferError> {
        let data = frame.data();
        let len = data.len();
        let checksum = frame.checksum();
        let mut buf = [0; TX_CHUNK];
        let mut n = 0;
        for i in 0..len {
            let (cmd, octet) = match i {
                0 => (commands::U_L_DATA_START_REQ, data[0]),
                i if i == len - 1 => (commands::U_L_DATA_END_REQ | i as u8, checksum),
                i => (commands::U_L_DATA_CONT_REQ | i as u8, data[i]),
            };
            buf[n] = cmd;
            buf[n + 1] = octet;
            n += 2;
            if n == buf.len() || i == len - 1 {
                self.uart.write_all(&buf[..n]).await.map_err(io_error)?;
                n = 0;
            }
        }
        let mut cmd = [0; 1];
        self.read_with_timeout(frame.mut_data()).await?;
        self.read_byte(&mut cmd).await?;
        if cmd[0] & 0x7f != 0xb {
            error!("Invalid L_Data.con: {:x}", cmd[0]);
            return Err(TransferError::InvalidData(cmd[0]));
        }
        let con_status = if (cmd[0] >> 7) != 0 {
            ConStatus::Ok
        } else {
            ConStatus::NotOk
        };
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
            || bytes_read != frame.length()
        {
            return Err(TransferError::FrameError(FrameError::InvalidLength));
        }
        // The checksum makes the XOR over all octets, including itself, 0xFF.
        if frame.data()[..bytes_read].iter().fold(0, |acc, b| acc ^ b) != 0xFF {
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
            _ => unreachable!(),
        }
    }

    async fn ack_if_addressed(&mut self, dst_addr: &Address) -> Result<(), TransferError> {
        if match dst_addr {
            Address::Individual(ref addr) => addr == &crate::settings::ADDRESS,
            Address::Group(_) => true,
        } {
            self.ack(AckTypes::ACK).await
        } else {
            Ok(())
        }
    }

    /// Reads until `buf` is full or the line has been silent for `BUS_SILENCE_US`.
    async fn read_with_timeout(&mut self, buf: &mut [u8]) -> Result<usize, TransferError> {
        let mut read = 0;
        while read < buf.len() {
            let n = self
                .uart
                .read(&mut buf[read..])
                .with_timeout(Duration::from_micros(Self::BUS_SILENCE_US))
                .await
                .map_err(|_| TransferError::TimeoutError(read))?
                .map_err(io_error)?;
            if n == 0 {
                return Err(TransferError::ReadError(ErrorKind::BrokenPipe));
            }
            read += n;
        }
        Ok(read)
    }

    async fn read_with_timeout_ignore_errors(&mut self) -> usize {
        let mut buf = [0; 16];
        let mut read = 0;
        warn!("Dumping data until timeout ignoreing errors...");
        loop {
            match self
                .uart
                .read(&mut buf)
                .with_timeout(Duration::from_micros(Self::BUS_SILENCE_US))
                .await
            {
                Ok(Ok(0)) | Err(_) => return read,
                Ok(Ok(n)) => read += n,
                Ok(Err(_)) => {}
            }
        }
    }
}
