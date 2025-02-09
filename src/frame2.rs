use core::{
    ops::{Deref, Range},
    panic,
};
use defmt::*;
use heapless::pool::boxed::Box;

use crate::ncn51_driver::FRAME_POOL;

type Field = Range<usize>;

#[derive(Debug, Format)]
pub enum FrameParseError {
    SizeError,
    ChecksumError(u8, u8),
}

#[derive(Format, Clone)]
pub enum Priority {
    Low = 0x3,
    Normal = 0x1,
    Urgent = 0x2,
    System = 0x0,
}
impl Priority {
    fn from_ctrl(ctrl: u8) -> Self {
        match (ctrl >> 2) & 0x3 {
            0x0 => Priority::System,
            0x1 => Priority::Normal,
            0x2 => Priority::Urgent,
            0x3 => Priority::Low,
            _ => panic!(),
        }
    }
}

#[derive(Format, Clone)]
enum Repeated {
    Repeated = 0,
    NotRepeated = 1,
}

impl Repeated {
    fn from_ctrl(ctrl: u8) -> Self {
        match (ctrl >> 5) & 0b1 {
            0 => Repeated::Repeated,
            1 => Repeated::NotRepeated,
            _ => panic!(),
        }
    }
}

#[derive(Format, Clone)]
pub enum AddressType {
    Individual = 0,
    Group = 1,
}

impl AddressType {
    fn from_byte(byte: u8) -> Self {
        match (byte >> 7) & 0b1 {
            0 => AddressType::Individual,
            1 => AddressType::Group,
            _ => panic!(),
        }
    }
}

#[derive(Format, PartialEq, PartialOrd, Clone)]
pub enum FrameType {
    Standard = 1,
    Extended = 2,
    Poll,
    Ack,
}

pub struct Header {
    frame_type: FrameType,
    repeated: Repeated,
    priority: Priority,
    src_addr: Address,
    dst_addr: Address,
    hop_count: u8,
    length: usize,
}

impl Header {
    const CTRL: usize = 0;
    const CTRLE: usize = 1;
    const AT_NPCI_LG: usize = 5;
    const STD_SA: Field = 1..3;
    const STD_DA: Field = 3..5;
    const LG: usize = 6;
    const EXT_SA: Field = 2..4;
    const EXT_DA: Field = 4..6;

    pub fn parse_std(buf: &[u8]) -> Result<Self, FrameParseError> {
        let length = buf[5] as usize & 0xF;
        if length > 14 {
            return Err(FrameParseError::SizeError);
        }
        Ok(Self {
            frame_type: FrameType::Standard,
            repeated: Repeated::from_ctrl(buf[Self::CTRL]),
            priority: Priority::from_ctrl(buf[Self::CTRL]),
            src_addr: Address::Individual(IndividualAddress::from(&buf[Self::STD_SA])),
            dst_addr: match AddressType::from_byte(buf[Self::AT_NPCI_LG]) {
                AddressType::Individual => {
                    Address::Individual(IndividualAddress::from(&buf[Self::STD_DA]))
                }
                AddressType::Group => Address::Group(GroupAddress::from(&buf[Self::STD_DA])),
            },
            hop_count: (buf[Self::AT_NPCI_LG] >> 4) & 0x7,
            length: length,
        })
    }
    pub fn parse_ext(buf: &[u8]) -> Result<Self, FrameParseError> {
        let length = buf[6] as usize;
        if length > 254 {
            return Err(FrameParseError::SizeError);
        }
        Ok(Self {
            frame_type: FrameType::Extended,
            repeated: Repeated::from_ctrl(buf[Self::CTRL]),
            priority: Priority::from_ctrl(buf[Self::CTRL]),
            src_addr: Address::Individual(IndividualAddress::from(&buf[Self::EXT_SA])),
            dst_addr: match AddressType::from_byte(buf[Self::CTRLE]) {
                AddressType::Individual => {
                    Address::Individual(IndividualAddress::from(&buf[Self::EXT_DA]))
                }
                AddressType::Group => Address::Group(GroupAddress::from(&buf[Self::EXT_SA])),
            },
            hop_count: (buf[Self::CTRLE] >> 4) & 0x7,
            length: buf[Self::LG] as usize,
        })
    }

    pub fn length(&self) -> usize {
        match self.frame_type {
            FrameType::Standard => 7,
            FrameType::Extended => 8,
            _ => panic!(),
        }
    }

    pub fn frame_length(&self) -> usize {
        self.length
    }

    pub fn total_length(&self) -> usize {
        self.length() + self.frame_length()
    }

    pub fn src_addr(&self) -> &Address {
        &self.src_addr
    }

    pub fn dst_addr(&self) -> &Address {
        &self.dst_addr
    }

    pub fn frame_type(&self) -> &FrameType {
        &self.frame_type
    }
}

impl Format for Header {
    fn format(&self, fmt: Formatter<'_>) {
        defmt::write!(fmt, "Frame type: {}\n", self.frame_type);
        defmt::write!(fmt, "Priority: {}\n", self.priority);
        defmt::write!(fmt, "Source Address: {}\n", self.src_addr);
        defmt::write!(fmt, "Desination Address: {}\n", self.dst_addr);
        defmt::write!(fmt, "Frame length: {}", self.length);
    }
}

pub struct Frame {
    pub header: Header,
    pub data: Box<FRAME_POOL>,
}

impl Format for Frame {
    fn format(&self, fmt: Formatter<'_>) {
        self.header.format(fmt);
        defmt::write!(fmt, "Data: {:#04X}", self.data.as_slice());
    }
}

impl Deref for Frame {
    type Target = Header;

    fn deref(&self) -> &Self::Target {
        &self.header
    }
}

impl Frame {
    fn validate_checksum(&self, chk: u8) -> Result<(), FrameParseError> {
        let sum = self.checksum();
        if sum == chk {
            Ok(())
        } else {
            Err(FrameParseError::ChecksumError(chk, sum))
        }
    }

    pub fn checksum(&self) -> u8 {
        !self.data.iter().fold(0, |acc, &v| acc ^ v)
    }

    pub fn new(
        src_addr: IndividualAddress,
        dst_addr: Address,
        priority: Priority,
        hop_count: u8,
        data: Box<FRAME_POOL>,
    ) -> Self {
        let frame_type = if data.len() > 24 {
            FrameType::Extended
        } else {
            FrameType::Standard
        };
        Self {
            header: Header {
                frame_type: frame_type.clone(),
                repeated: Repeated::NotRepeated,
                priority: priority,
                src_addr: Address::Individual(src_addr),
                dst_addr: dst_addr,
                hop_count: hop_count,
                length: data.len() - 8,
            },
            data: data,
        }
    }

    pub fn new_with_check(
        header: Header,
        data: Box<FRAME_POOL>,
        chk: u8,
    ) -> Result<Self, FrameParseError> {
        let frame = Self {
            header: header,
            data: data,
        };
        frame.validate_checksum(chk)?;
        Ok(frame)
    }

    pub fn data(&self) -> &[u8] {
        &self.data[self.header.length() - 1..]
    }

    fn encode_ctrl(&mut self, i: usize) {
        let c = &mut self.data[i];
        *c = 0;
        *c |= (self.header.frame_type.clone() as u8) << 7;
        *c |= (self.header.repeated.clone() as u8) << 4;
        *c |= (self.header.priority.clone() as u8) << 2;
    }

    fn encode_ctrle(&mut self, i: usize) {
        let mut c = 0;
        if let Address::Group(_) = self.dst_addr {
            c |= 1 << 7;
        }
        c |= self.header.hop_count << 4;
        self.data[i] = c;
    }

    pub fn encode<'a>(&'a mut self) -> &'a [u8] {
        match self.header.frame_type {
            FrameType::Standard => {
                self.encode_ctrl(1);
                self.header.src_addr.write(&mut self.data[2..4]);
                self.header.dst_addr.write(&mut self.data[4..6]);
                self.data[6] = 0;
                if let Address::Group(_) = self.dst_addr {
                    self.data[6] |= 1 << 7;
                }
                self.data[6] |= self.hop_count << 4;
                self.data[6] |= self.header.frame_length() as u8 & 0xF;
                &mut &self.data[1..]
            }
            FrameType::Extended => {
                self.encode_ctrl(0);
                self.encode_ctrle(1);
                self.header.src_addr.write(&mut self.data[2..4]);
                self.header.dst_addr.write(&mut self.data[4..6]);
                self.data[6] = self.header.frame_length() as u8;
                &mut &self.data[..]
            }
            _ => panic!(),
        }
    }
}
