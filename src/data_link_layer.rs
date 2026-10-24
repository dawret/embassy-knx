use crate::bus::{Bus, ConStatus};
use crate::frame::*;

pub struct DataLinkLayer {
    bus: &'static Bus,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DataServiceInd {
    Data(Frame),
    SystemBroadcast(Frame),
    Busmon(Frame),
    ServiceInformation(Frame),
}

pub enum DataServiceCon {
    Data(ConStatus),
}

impl DataLinkLayer {
    pub fn new(bus: &'static Bus) -> Self {
        Self { bus }
    }

    pub async fn send(&self, frame: Frame) -> (Frame, ConStatus) {
        self.bus.con.reset();
        self.bus.bus_tx.send(frame).await;
        self.bus.con.wait().await
    }

    /// Cancel-safe: only takes the next frame off the queue.
    pub async fn next_frame(&self) -> Frame {
        self.bus.bus_rx.receive().await
    }

    /// Individually addressed frames for other devices are not indicated.
    pub fn receive(&self, frame: Frame) -> Option<DataServiceInd> {
        match frame.dst_addr() {
            Address::Individual(addr) if addr != crate::settings::ADDRESS => None,
            _ => Some(DataServiceInd::Data(frame)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::frame;

    fn data_link() -> DataLinkLayer {
        DataLinkLayer::new(Box::leak(Box::new(Bus::new())))
    }

    #[test]
    fn frame_for_another_device_is_not_indicated() {
        let dl = data_link();
        let t_connect = frame(&[0xB0, 0x11, 0xFA, 0x11, 0x05, 0x60, 0x80]);
        assert!(dl.receive(t_connect).is_none());
    }

    #[test]
    fn frames_for_us_and_group_frames_are_indicated() {
        let dl = data_link();
        let t_connect = frame(&[0xB0, 0x11, 0xFA, 0x11, 0x78, 0x60, 0x80]);
        let group_read = frame(&[0xBC, 0x11, 0xFA, 0x09, 0x62, 0xE1, 0x00, 0x00]);
        for f in [t_connect, group_read] {
            let ind = dl.receive(f);
            assert!(matches!(ind, Some(DataServiceInd::Data(_))), "{ind:?}");
        }
    }
}
