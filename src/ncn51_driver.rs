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
    Timeout(usize),
    Io(ErrorKind),
    Frame(FrameError),
}

fn io_error<E: embedded_io_async::Error>(err: E) -> TransferError {
    TransferError::Io(err.kind())
}

impl From<FrameError> for TransferError {
    fn from(err: FrameError) -> Self {
        TransferError::Frame(err)
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
    Nack = 0x4,
    Busy = 0x2,
    Ack = 0x1,
}

const CHANNEL_SIZE: usize = 8;
const TX_CHUNK: usize = 64;
const L_DATA_CON: u8 = 0x0B;
const L_DATA_CON_MASK: u8 = 0x7F;
const REPEAT_FLAG: u8 = 0x20;
pub static CTRL_CHANNEL: Channel<CriticalSectionRawMutex, CtrlMsg, CHANNEL_SIZE> = Channel::new();

fn is_echo_header(header: &[u8], tx: &[u8]) -> bool {
    (header[0] ^ tx[0]) & !REPEAT_FLAG == 0 && header[1..] == tx[1..header.len()]
}

pub struct NCN51Driver<U, L> {
    uart: U,
    led: L,
    bus: &'static Bus,
}

impl<U: Read + Write, L: StatefulOutputPin> NCN51Driver<U, L> {
    const BUS_SILENCE_US: u64 = 2600;
    // Up to 7 attempts (NACK + BUSY repetitions) of a 263-octet frame, each about 400 ms.
    const CON_TIMEOUT_MS: u64 = 3000;
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
            0 => Err(TransferError::Io(ErrorKind::BrokenPipe)),
            _ => Ok(()),
        }
    }

    pub async fn run(mut self) -> ! {
        loop {
            let mut buf = [0; 1];
            let bus = self.bus;
            match select(self.read_byte(&mut buf), bus.bus_tx.ready_to_receive()).await {
                Either::First(Err(e)) => info!("Reception error: {}", e),
                Either::First(Ok(())) if self.is_frame_start(buf[0]) => {
                    self.receive(buf[0], None).await
                }
                Either::First(Ok(())) => info!("Unexpected byte: {:x}", buf[0]),
                Either::Second(_) => {
                    let mut frame = self.bus.bus_tx.receive().await;
                    let status = match self.send_frame(&mut frame).await {
                        Ok(status) => status,
                        Err(e) => {
                            error!("Transmission error: {}", e);
                            ConStatus::NotOk
                        }
                    };
                    self.bus.con.signal((frame, status));
                }
            }
        }
    }

    /// Passes the frame that starts with `ctrl` up, unless it is an echo of `tx`.
    async fn receive(&mut self, ctrl: u8, tx: Option<&Frame>) {
        match self.receive_frame(ctrl, tx).await {
            Ok(Some(frame)) => {
                let _ = self.led.toggle();
                self.bus.bus_rx.send(frame).await;
            }
            Ok(None) => {}
            Err(e) => {
                info!("Reception error: {}", e);
                self.read_with_timeout_ignore_errors().await;
                //TODO: Handle NACK

                /*if let Err(e) = self.ack(AckTypes::Nack).await {
                    info!("Failed to send NACK: {}", e);
                }*/
            }
        }
    }

    async fn send_frame(&mut self, frame: &mut Frame) -> Result<ConStatus, TransferError> {
        let len = frame.data().len();
        frame.mut_data()[len - 1] = frame.checksum();
        let mut buf = [0; TX_CHUNK];
        let mut n = 0;
        for (i, &octet) in frame.data().iter().enumerate() {
            // The commands carry 6 index bits; U_L_DataOffset sets the upper 3.
            if i > 0 && i % 64 == 0 {
                buf[n] = commands::U_L_DATA_OFFSET_REQ | (i / 64) as u8;
                n += 1;
            }
            let index = (i % 64) as u8;
            buf[n] = match i {
                0 => commands::U_L_DATA_START_REQ,
                i if i == len - 1 => commands::U_L_DATA_END_REQ | index,
                _ => commands::U_L_DATA_CONT_REQ | index,
            };
            buf[n + 1] = octet;
            n += 2;
            if n + 3 > buf.len() || i == len - 1 {
                self.uart.write_all(&buf[..n]).await.map_err(io_error)?;
                n = 0;
            }
        }
        self.confirmation(frame)
            .with_timeout(Duration::from_millis(Self::CON_TIMEOUT_MS))
            .await
            .map_err(|_| TransferError::Timeout(0))?
    }

    /// The transceiver echoes every attempt to send `tx`. Frames of devices that win the
    /// arbitration arrive in between.
    async fn confirmation(&mut self, tx: &Frame) -> Result<ConStatus, TransferError> {
        loop {
            let mut buf = [0; 1];
            self.read_byte(&mut buf).await?;
            match buf[0] {
                b if b & L_DATA_CON_MASK == L_DATA_CON => {
                    return Ok(if b & 0x80 != 0 {
                        ConStatus::Ok
                    } else {
                        ConStatus::NotOk
                    });
                }
                b if self.is_frame_start(b) => self.receive(b, Some(tx)).await,
                b => info!("Unexpected byte: {:x}", b),
            }
        }
    }

    /// Reads exactly the length given in the header, so the frame is passed up without
    /// waiting for bus silence.
    async fn receive_frame(
        &mut self,
        ctrl: u8,
        tx: Option<&Frame>,
    ) -> Result<Option<Frame>, TransferError> {
        let frame_type = FrameType::of(ctrl);
        let mut header = [0; FrameType::Extended.header_length()];
        let header = &mut header[..frame_type.header_length()];
        header[0] = ctrl;
        self.read_with_timeout(&mut header[1..]).await?;
        if let Some(tx) = tx.map(Frame::data).filter(|tx| is_echo_header(header, tx)) {
            self.receive_echo(&tx[header.len()..]).await?;
            return Ok(None);
        }
        let mut frame = Frame::new(frame_type, Frame::length_in_header(header))?;
        frame.mut_data()[..header.len()].copy_from_slice(header);
        self.ack_if_addressed(&frame.dst_addr()).await?;
        self.read_with_timeout(&mut frame.mut_data()[header.len()..])
            .await?;
        // The checksum makes the XOR over all octets, including itself, 0xFF.
        if frame.data().iter().fold(0, |acc, b| acc ^ b) != 0xFF {
            return Err(TransferError::Frame(FrameError::Checksum));
        }
        Ok(Some(frame))
    }

    /// `expected` ends with the checksum octet, which differs in repetitions.
    async fn receive_echo(&mut self, expected: &[u8]) -> Result<(), TransferError> {
        let (body, _) = expected.split_at(expected.len() - 1);
        let mut buf = [0; 16];
        let mut matches = true;
        for chunk in body.chunks(buf.len()) {
            let echo = &mut buf[..chunk.len()];
            self.read_with_timeout(echo).await?;
            matches &= echo == chunk;
        }
        self.read_with_timeout(&mut buf[..1]).await?;
        if !matches {
            warn!("Echo differs from the sent frame");
        }
        Ok(())
    }

    async fn ack_if_addressed(&mut self, dst_addr: &Address) -> Result<(), TransferError> {
        if match dst_addr {
            Address::Individual(ref addr) => addr == &crate::settings::ADDRESS,
            Address::Group(_) => true,
        } {
            self.ack(AckTypes::Ack).await
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
                .map_err(|_| TransferError::Timeout(read))?
                .map_err(io_error)?;
            if n == 0 {
                return Err(TransferError::Io(ErrorKind::BrokenPipe));
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
