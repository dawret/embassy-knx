#![allow(dead_code)]

use core::convert::Infallible;
use core::future::Future;
use embassy_futures::select::{select, Either};
use embassy_knx::frame::{init_frame_pool, LargeFrameBlock, SmallFrameBlock};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;
use embassy_time::{Duration, Timer, WithTimeout};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;

pub const U_ACK: u8 = 0x11;
pub const ME: [u8; 2] = [0x11, 0x78];

pub fn init_pool() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let small = Box::leak(Box::new([const { SmallFrameBlock::new() }; 64]));
        let large = Box::leak(Box::new([const { LargeFrameBlock::new() }; 64]));
        init_frame_pool(small, large);
    });
}

pub fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}

pub fn leak_slice<T: Clone>(values: &[T]) -> &'static [T] {
    Box::leak(values.to_vec().into_boxed_slice())
}

pub fn run<F: Future>(fut: F) -> F::Output {
    futures_executor::block_on(fut.with_timeout(Duration::from_secs(2))).expect("test timed out")
}

pub async fn with_background<F: Future, B: Future>(body: F, background: B) -> F::Output {
    match select(body, background).await {
        Either::First(out) => out,
        Either::Second(_) => panic!("background task finished"),
    }
}

pub fn with_checksum(telegram: &[u8]) -> Vec<u8> {
    let mut bytes = telegram.to_vec();
    bytes.push(!telegram.iter().fold(0, |acc, b| acc ^ b));
    bytes
}

pub struct Ncn5130Sim {
    to_host: Pipe<CriticalSectionRawMutex, 1024>,
    from_host: Pipe<CriticalSectionRawMutex, 1024>,
    writes: AtomicUsize,
}

impl Ncn5130Sim {
    pub fn new() -> Self {
        Self {
            to_host: Pipe::new(),
            from_host: Pipe::new(),
            writes: AtomicUsize::new(0),
        }
    }

    pub fn uart(&self) -> SimUart<'_> {
        SimUart { sim: self }
    }

    pub fn send_to_host(&self, bytes: &[u8]) {
        assert_eq!(
            self.to_host.try_write(bytes),
            Ok(bytes.len()),
            "UART buffer full"
        );
    }

    pub async fn bus_receive(&self, telegram: &[u8]) {
        self.send_to_host(&with_checksum(telegram));
        self.bus_idle().await;
    }

    /// Long enough for the driver's end-of-frame timeout (2.6 ms) to fire.
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

    pub fn host_bytes_now(&self) -> Vec<u8> {
        let mut buf = [0; 1024];
        let n = self.from_host.try_read(&mut buf).unwrap_or(0);
        buf[..n].to_vec()
    }

    pub fn host_writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }

    pub async fn expect_ack(&self) {
        assert_eq!(
            self.host_byte().await,
            U_ACK,
            "expected U_AckInformation(ACK)"
        );
    }

    pub async fn transmit(&self, con_ok: bool) -> Vec<u8> {
        let telegram = self.host_request().await;
        self.echo_and_confirm(&telegram, con_ok);
        telegram
    }

    /// Decodes one U_L_DataStart/Continue/End request; returns the telegram with its checksum.
    pub async fn host_request(&self) -> Vec<u8> {
        assert_eq!(self.host_byte().await, 0x80, "expected U_L_DataStart");
        let mut telegram = vec![self.host_byte().await];
        let mut offset = 0;
        loop {
            let cmd = self.host_byte().await;
            // U_L_DataOffset: the 3 high bits of the 9-bit index.
            if cmd & 0xF8 == 0x08 {
                offset = (cmd & 0x07) as usize * 64;
                continue;
            }
            let index = offset + (cmd & 0x3F) as usize;
            assert_eq!(index, telegram.len(), "command {cmd:#04X} out of sequence");
            match cmd & 0xC0 {
                // U_L_DataContinue
                0x80 => telegram.push(self.host_byte().await),
                // U_L_DataEnd: the next octet is the checksum.
                0x40 => {
                    telegram.push(self.host_byte().await);
                    break;
                }
                _ => panic!("unexpected command {cmd:#04X}"),
            }
        }
        telegram
    }

    pub fn echo_and_confirm(&self, telegram: &[u8], con_ok: bool) {
        self.send_to_host(telegram);
        self.send_to_host(&[if con_ok { 0x8B } else { 0x0B }]);
    }
}

/// The telegram as the transceiver repeats it: repeat bit cleared, new checksum.
pub fn repetition(telegram: &[u8]) -> Vec<u8> {
    let mut repeated = telegram[..telegram.len() - 1].to_vec();
    repeated[0] &= !0x20;
    with_checksum(&repeated)
}

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
        self.sim.writes.fetch_add(1, Ordering::Relaxed);
        Ok(self.sim.from_host.write(buf).await)
    }

    async fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

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

pub async fn settle() {
    Timer::after(Duration::from_millis(20)).await;
}
