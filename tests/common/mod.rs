//! Shared helpers for the integration tests: frame pool setup, a time-limited executor and
//! [`Ncn5130Sim`], a simulated NCN5130 transceiver.
#![allow(dead_code)]

use core::convert::Infallible;
use core::future::Future;
use embassy_futures::select::{select, Either};
use embassy_knx::frame::{init_frame_pool, FrameBlock};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;
use embassy_time::{Duration, Timer, WithTimeout};
use std::sync::Once;

/// U_AckInformation with the ACK bit set.
pub const U_ACK: u8 = 0x11;
/// Our address, `settings::ADDRESS` (1.1.120).
pub const ME: [u8; 2] = [0x11, 0x78];

pub fn init_pool() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let blocks = Box::leak(Box::new([const { FrameBlock::new() }; 64]));
        init_frame_pool(blocks);
    });
}

pub fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}

/// Runs `fut` to completion and fails the test if that takes longer than two seconds.
pub fn run<F: Future>(fut: F) -> F::Output {
    futures_executor::block_on(fut.with_timeout(Duration::from_secs(2))).expect("test timed out")
}

/// Runs `body` while `background` (e.g. the driver's endless loop) keeps running.
pub async fn with_background<F: Future, B: Future>(body: F, background: B) -> F::Output {
    match select(body, background).await {
        Either::First(out) => out,
        Either::Second(_) => panic!("background task finished"),
    }
}

/// Appends the KNX checksum (inverted XOR of all octets) to a telegram.
pub fn with_checksum(telegram: &[u8]) -> Vec<u8> {
    let mut bytes = telegram.to_vec();
    bytes.push(!telegram.iter().fold(0, |acc, b| acc ^ b));
    bytes
}

/// A simulated NCN5130: the bus side is driven by the test, the UART side by the driver
/// through [`SimUart`].
pub struct Ncn5130Sim {
    /// Transceiver → host.
    to_host: Pipe<CriticalSectionRawMutex, 1024>,
    /// Host → transceiver.
    from_host: Pipe<CriticalSectionRawMutex, 1024>,
}

impl Ncn5130Sim {
    pub fn new() -> Self {
        Self {
            to_host: Pipe::new(),
            from_host: Pipe::new(),
        }
    }

    pub fn uart(&self) -> SimUart<'_> {
        SimUart { sim: self }
    }

    /// Sends raw bytes to the host. They arrive back to back, as a telegram does on the UART:
    /// the driver's end-of-frame timeout only fires after the last one.
    pub fn send_to_host(&self, bytes: &[u8]) {
        assert_eq!(self.to_host.try_write(bytes), Ok(bytes.len()), "UART buffer full");
    }

    /// A telegram (given without checksum) arrives from the bus, followed by bus idle time.
    pub async fn bus_receive(&self, telegram: &[u8]) {
        self.send_to_host(&with_checksum(telegram));
        self.bus_idle().await;
    }

    /// Waits until the host has read everything, then long enough for its end-of-frame
    /// timeout (2.6 ms) to fire, as the gap between telegrams on a real bus would.
    pub async fn bus_idle(&self) {
        while !self.to_host.is_empty() {
            Timer::after(Duration::from_micros(100)).await;
        }
        Timer::after(Duration::from_millis(10)).await;
    }

    pub async fn host_byte(&self) -> u8 {
        let mut byte = [0];
        self.from_host.read(&mut byte).await;
        byte[0]
    }

    /// Everything the host has written so far, without waiting.
    pub fn host_bytes_now(&self) -> Vec<u8> {
        let mut buf = [0; 1024];
        let n = self.from_host.try_read(&mut buf).unwrap_or(0);
        buf[..n].to_vec()
    }

    pub async fn expect_ack(&self) {
        assert_eq!(self.host_byte().await, U_ACK, "expected U_AckInformation(ACK)");
    }

    /// Serves one L_Data request from the host: decodes the U_L_DataStart/Continue/End
    /// command stream, echoes the telegram as the bus would and answers with L_Data.con.
    /// Returns the telegram, including the checksum the host computed.
    pub async fn transmit(&self, con_ok: bool) -> Vec<u8> {
        assert_eq!(self.host_byte().await, 0x80, "expected U_L_DataStart");
        let mut telegram = vec![self.host_byte().await];
        loop {
            let cmd = self.host_byte().await;
            let index = (cmd & 0x3F) as usize;
            assert_eq!(index, telegram.len(), "command {cmd:#04X} out of sequence");
            match cmd & 0xC0 {
                // U_L_DataContinue: the next octet goes to `index`.
                0x80 => telegram.push(self.host_byte().await),
                // U_L_DataEnd: the next octet is the checksum.
                0x40 => {
                    telegram.push(self.host_byte().await);
                    break;
                }
                _ => panic!("unexpected command {cmd:#04X}"),
            }
        }
        self.send_to_host(&telegram);
        self.send_to_host(&[if con_ok { 0x8B } else { 0x0B }]);
        telegram
    }
}

/// The driver's end of the simulated UART.
pub struct SimUart<'a> {
    sim: &'a Ncn5130Sim,
}

impl embedded_io_async::ErrorType for SimUart<'_> {
    type Error = Infallible;
}

impl embedded_io_async::Read for SimUart<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Infallible> {
        Ok(self.sim.to_host.read(buf).await)
    }
}

impl embedded_io_async::Write for SimUart<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Infallible> {
        Ok(self.sim.from_host.write(buf).await)
    }

    async fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

/// Stand-in for the RX LED.
pub struct NoLed;

impl embedded_hal::digital::ErrorType for NoLed {
    type Error = Infallible;
}

impl embedded_hal::digital::OutputPin for NoLed {
    fn set_low(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl embedded_hal::digital::StatefulOutputPin for NoLed {
    fn is_set_high(&mut self) -> Result<bool, Infallible> {
        Ok(false)
    }
    fn is_set_low(&mut self) -> Result<bool, Infallible> {
        Ok(true)
    }
}

/// Waits briefly, so that bytes the driver might still write become visible.
pub async fn settle() {
    Timer::after(Duration::from_millis(20)).await;
}
