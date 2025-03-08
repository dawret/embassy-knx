use crate::data_point::{DataPoint, DataPointAccess, DataPointLength};
use crate::ncn51_driver::FRAME_POOL;
use defmt::*;
use heapless::{pool::boxed::Box, Vec};
use num_enum::{IntoPrimitive, UnsafeFromPrimitive};

#[derive(Format, UnsafeFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum FrameType {
    Standard = 1,
    Extended = 0,
}

#[derive(Format)]
pub enum FrameError {
    InvalidLength,
    OutOfMemory,
    Checksum,
    InvalidTpdu(u8),
}

type FrameResult<T> = Result<T, FrameError>;

#[derive(Format, UnsafeFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum AddressType {
    Individual = 0,
    Group = 1,
}

#[derive(Format, UnsafeFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum Priority {
    Low = 0x3,
    Normal = 0x1,
    Urgent = 0x2,
    System = 0x0,
}

#[derive(Format, UnsafeFromPrimitive, IntoPrimitive)]
#[repr(u8)]
enum Repeated {
    Repeated = 0,
    NotRepeated = 1,
}

#[derive(PartialEq, PartialOrd, Hash, Clone)]
pub struct IndividualAddress(u16);

impl IndividualAddress {
    pub const fn from_parts(n1: u8, n2: u8, n3: u8) -> Self {
        IndividualAddress((n1 as u16) << 12 | (n2 as u16) << 8 | n3 as u16)
    }
    pub const fn new(addr: u16) -> Self {
        IndividualAddress(addr)
    }
    pub fn write(&self, buf: &mut [u8]) {
        buf[0] = (self.0 >> 8) as u8;
        buf[1] = (self.0 & 0xFF) as u8;
    }
}

impl From<&[u8]> for IndividualAddress {
    fn from(buf: &[u8]) -> Self {
        Self(((buf[0] as u16) << 8) | buf[1] as u16)
    }
}

impl From<u16> for IndividualAddress {
    fn from(addr: u16) -> Self {
        Self(addr)
    }
}

impl Format for IndividualAddress {
    fn format(&self, fmt: Formatter) {
        defmt::write!(
            fmt,
            "{}.{}.{}",
            self.0 >> 12,
            (self.0 >> 8) & 0xF,
            self.0 & 0xFF
        );
    }
}

#[derive(PartialEq, PartialOrd, Hash)]
pub struct GroupAddress(u16);

impl GroupAddress {
    pub const fn from_parts(n1: u8, n2: u8, n3: u8) -> Self {
        GroupAddress((n1 as u16) << 11 | (n2 as u16) << 8 | n3 as u16)
    }
    pub const fn new(addr: u16) -> Self {
        GroupAddress(addr)
    }
    pub fn write(&self, buf: &mut [u8]) {
        buf[0] = (self.0 >> 8) as u8;
        buf[1] = (self.0 & 0xFF) as u8;
    }
}

impl Format for GroupAddress {
    fn format(&self, fmt: Formatter) {
        defmt::write!(
            fmt,
            "{}/{}/{}",
            self.0 >> 11,
            (self.0 >> 8) & 0x7,
            self.0 & 0xFF
        );
    }
}

impl From<&[u8]> for GroupAddress {
    fn from(buf: &[u8]) -> Self {
        Self(((buf[0] as u16) << 8) | buf[1] as u16)
    }
}

impl From<u16> for GroupAddress {
    fn from(addr: u16) -> Self {
        Self(addr)
    }
}

#[derive(Format)]
pub enum Address {
    Individual(IndividualAddress),
    Group(GroupAddress),
}

impl Address {
    pub fn write(&self, buf: &mut [u8]) {
        match self {
            Address::Individual(ref a) => a.write(buf),
            Address::Group(ref a) => a.write(buf),
        }
    }
}

#[derive(Format, IntoPrimitive)]
#[repr(usize)]
pub enum TpciBits {
    Six = 6,
    Eight = 8,
}
#[derive(Format, IntoPrimitive)]
#[repr(usize)]
pub enum ApciBits {
    Four = 6,
    Ten = 8,
}

pub trait FrameReader {
    const CTRL: usize = 0;
    const CTRL_OFFSET: usize;
    const SRC_ADDR_OFFSET: usize = Self::CTRL_OFFSET + 0;
    const DST_ADDR_OFFSET: usize = Self::CTRL_OFFSET + 2;
    const HEADER_LENGTH: usize = Self::CTRL_OFFSET + 6;
    const APCI_OFFSET: usize = Self::HEADER_LENGTH - 1;
    const APCI_BASE_SIZE: usize = 1;
    const LG_FIELD: usize = 5;
    const AT_FIELD: usize;
    const HOP_COUNT_FIELD: usize;
    const TPCI_OFFSET: usize = Self::HEADER_LENGTH - 1;
    const MAX_FRAME_SIZE: usize;
    const MIN_FRAME_SIZE: usize;

    fn data(&self) -> &[u8];
    fn length(&self) -> u8;
    fn frame_type(&self) -> FrameType {
        // This is safe, possible values: 0, 1
        unsafe { FrameType::unchecked_transmute_from(self.data()[Self::CTRL] >> 7) }
    }
    fn addr_type(&self) -> AddressType {
        // This is safe, possible values: 0, 1
        unsafe { AddressType::unchecked_transmute_from(self.data()[Self::AT_FIELD] >> 7) }
    }
    fn src_addr(&self) -> IndividualAddress {
        IndividualAddress::from(&self.data()[Self::SRC_ADDR_OFFSET..])
    }
    fn dst_addr(&self) -> Address {
        let dst_addr = &self.data()[Self::DST_ADDR_OFFSET..];
        match self.addr_type() {
            AddressType::Group => Address::Group(GroupAddress::from(dst_addr)),
            AddressType::Individual => Address::Individual(IndividualAddress::from(dst_addr)),
        }
    }
    fn hop_count(&self) -> u8 {
        (self.data()[Self::HOP_COUNT_FIELD] >> 4) & 0x7
    }
    fn priority(&self) -> Priority {
        // This is safe, possible values: 0, 1, 2, 3
        unsafe { Priority::unchecked_transmute_from((self.data()[Self::CTRL] >> 2) & 0x3) }
    }
    fn repeated(&self) -> Repeated {
        // This is safe, possible values: 0, 1
        unsafe { Repeated::unchecked_transmute_from((self.data()[Self::CTRL] >> 5) & 0x1) }
    }
    fn tpci(&self, bits: TpciBits) -> u8 {
        match bits {
            TpciBits::Six => self.data()[Self::TPCI_OFFSET] >> 2,
            TpciBits::Eight => self.data()[Self::TPCI_OFFSET],
        }
    }
    fn tpci_seq(&self) -> u8 {
        (self.data()[Self::TPCI_OFFSET] >> 2) & 0x7
    }
    fn apci(&self, bits: ApciBits) -> u16 {
        let apci: u16 = ((self.data()[Self::APCI_OFFSET] as u16 & 0x3) << 8)
            | self.data()[Self::APCI_OFFSET + 1] as u16;
        match bits {
            ApciBits::Four => apci >> 6,
            ApciBits::Ten => apci,
        }
    }
    fn apdu_data(&self) -> &[u8] {
        &self.data()[Self::APCI_OFFSET + 1..self.length() as usize]
    }

    fn checksum(&self) -> u8 {
        !self.data().iter().fold(0, |acc, &v| acc ^ v)
    }

    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(fmt, "Frame type: {}\n", self.frame_type());
        defmt::write!(fmt, "Priority: {}\n", self.priority());
        defmt::write!(fmt, "Source address: {}\n", self.src_addr());
        defmt::write!(fmt, "Destination address: {}\n", self.dst_addr());
        defmt::write!(fmt, "Frame length: {}\n", self.length());
        defmt::write!(fmt, "Data: {:#04X}", self.data());
    }
}

pub trait FrameWriter: FrameReader + Sized {
    fn mut_data(&mut self) -> &mut [u8];
    fn set_length(&mut self, length: usize) -> FrameResult<()>;
    fn set_frame_type(&mut self);
    fn new(size: usize) -> FrameResult<Self>;
    fn set_addr_type(&mut self, addr_type: AddressType) {
        let at: u8 = addr_type.into();
        self.mut_data()[Self::AT_FIELD] |= at << 7;
    }
    fn set_priority(&mut self, priority: Priority) {
        let p: u8 = priority.into();
        self.mut_data()[Self::CTRL] |= p << 2;
    }
    fn set_repeated(&mut self, repeated: Repeated) {
        let r: u8 = repeated.into();
        self.mut_data()[Self::CTRL] |= r << 5;
    }
    fn set_hop_count(&mut self, hop_count: u8) {
        self.mut_data()[Self::HOP_COUNT_FIELD] |= (hop_count & 0x7) << 4;
    }
    fn set_src_addr(&mut self, src_addr: &IndividualAddress) {
        src_addr.write(&mut self.mut_data()[Self::SRC_ADDR_OFFSET..]);
    }
    fn set_dst_addr(&mut self, dst_addr: &Address) {
        match dst_addr {
            Address::Individual(addr) => {
                self.set_addr_type(AddressType::Individual);
                addr.write(&mut self.mut_data()[Self::DST_ADDR_OFFSET..]);
            }
            Address::Group(addr) => {
                self.set_addr_type(AddressType::Group);
                addr.write(&mut self.mut_data()[Self::DST_ADDR_OFFSET..]);
            }
        }
    }
    fn from_datapoint(datapoint: &DataPoint) -> FrameResult<Self> {
        let size = match datapoint.length() {
            DataPointLength::Bit(n) => {
                if n < 6 {
                    0
                } else {
                    1
                }
            }
            DataPointLength::Byte(n) => n,
        };
        let mut frame = Self::new(Self::MAX_FRAME_SIZE)?;
        datapoint.write(
            &mut frame.mut_data()[Self::APCI_OFFSET + 1 + size..Self::APCI_OFFSET + size + 2],
        );
        frame.set_length(Self::HEADER_LENGTH + Self::APCI_BASE_SIZE + size + 1)?;
        Ok(frame)
    }
    fn set_tpci(&mut self, bits: TpciBits, val: u8) {
        match bits {
            TpciBits::Six => {
                self.mut_data()[Self::TPCI_OFFSET] |= val << 2;
            }
            TpciBits::Eight => self.mut_data()[Self::TPCI_OFFSET] = val,
        }
    }
    fn set_tpci_seq(&mut self, val: u8) {
        self.mut_data()[Self::TPCI_OFFSET] |= (val & 0x7) << 2;
    }
    fn set_apci(&mut self, bits: ApciBits, val: u16) {
        match bits {
            ApciBits::Four => {
                self.mut_data()[Self::APCI_OFFSET] |= (val >> 2) as u8;
                self.mut_data()[Self::APCI_OFFSET + 1] |= ((val & 0x3) << 6) as u8;
            }
            ApciBits::Ten => {
                self.mut_data()[Self::APCI_OFFSET] |= (val >> 8) as u8;
                self.mut_data()[Self::APCI_OFFSET + 1] |= (val & 0xFF) as u8;
            }
        }
    }
}

pub struct StandardFrame(Box<FRAME_POOL>);

impl FrameReader for StandardFrame {
    const CTRL_OFFSET: usize = 1;
    const AT_FIELD: usize = 5;
    const HOP_COUNT_FIELD: usize = 5;
    const MAX_FRAME_SIZE: usize = 24;
    const MIN_FRAME_SIZE: usize = 8;

    fn data(&self) -> &[u8] {
        &self.0
    }
    fn length(&self) -> u8 {
        (self.0[Self::LG_FIELD] & 0xF) + Self::HEADER_LENGTH as u8 + 1
    }
}

impl FrameWriter for StandardFrame {
    fn mut_data(&mut self) -> &mut [u8] {
        &mut self.0
    }
    fn set_length(&mut self, length: usize) -> FrameResult<()> {
        if length <= Self::MAX_FRAME_SIZE || length >= Self::MIN_FRAME_SIZE {
            self.0[Self::LG_FIELD] |= (length as u8 - Self::HEADER_LENGTH as u8 - 1) & 0xF;
            self.0
                .resize_default(length)
                .map_err(|_| FrameError::InvalidLength)?;
            Ok(())
        } else {
            Err(FrameError::InvalidLength)
        }
    }
    fn set_frame_type(&mut self) {
        self.0[Self::CTRL] |= 0x90;
    }
    fn new(size: usize) -> FrameResult<Self> {
        let mut buf = FRAME_POOL
            .alloc(Vec::new())
            .map_err(|_| FrameError::OutOfMemory)?;
        buf.resize_default(size)
            .map_err(|_| FrameError::InvalidLength)?;
        let mut frame = StandardFrame(buf);
        frame.set_length(size)?;
        frame.set_frame_type();
        Ok(frame)
    }
}

impl defmt::Format for StandardFrame {
    fn format(&self, fmt: Formatter) {
        FrameReader::format(self, fmt);
    }
}

impl TryFrom<&StandardFrame> for StandardFrame {
    type Error = FrameError;
    fn try_from(value: &StandardFrame) -> Result<Self, Self::Error> {
        let mut new_frame = StandardFrame::new(value.length() as usize)?;
        new_frame.0.clone_from_slice(value.data());
        Ok(new_frame)
    }
}

pub struct ExtendedFrame(Box<FRAME_POOL>);

impl FrameReader for ExtendedFrame {
    const CTRL_OFFSET: usize = 2;
    const AT_FIELD: usize = 1;
    const HOP_COUNT_FIELD: usize = 1;
    const MAX_FRAME_SIZE: usize = 263;
    const MIN_FRAME_SIZE: usize = 9;

    fn data(&self) -> &[u8] {
        &self.0
    }
    fn length(&self) -> u8 {
        self.0[Self::LG_FIELD] + Self::HEADER_LENGTH as u8 + 1
    }
}

impl FrameWriter for ExtendedFrame {
    fn mut_data(&mut self) -> &mut [u8] {
        &mut self.0
    }
    fn set_length(&mut self, length: usize) -> FrameResult<()> {
        if length <= Self::MAX_FRAME_SIZE || length >= Self::MIN_FRAME_SIZE {
            self.0[Self::LG_FIELD] = self.0.len() as u8 - Self::HEADER_LENGTH as u8 - 1;
            self.0
                .resize_default(length)
                .map_err(|_| FrameError::InvalidLength)?;
            Ok(())
        } else {
            Err(FrameError::InvalidLength)
        }
    }
    fn set_frame_type(&mut self) {
        self.0[Self::CTRL] |= 0x10;
    }
    fn new(size: usize) -> FrameResult<Self> {
        let mut buf = FRAME_POOL
            .alloc(Vec::new())
            .map_err(|_| FrameError::OutOfMemory)?;
        buf.resize_default(size)
            .map_err(|_| FrameError::InvalidLength)?;
        Ok(ExtendedFrame(buf))
    }
}

impl defmt::Format for ExtendedFrame {
    fn format(&self, fmt: Formatter) {
        FrameReader::format(self, fmt);
    }
}

impl TryFrom<&ExtendedFrame> for ExtendedFrame {
    type Error = FrameError;
    fn try_from(value: &ExtendedFrame) -> Result<Self, Self::Error> {
        let mut new_frame = ExtendedFrame::new(value.length() as usize)?;
        new_frame.0.clone_from_slice(value.data());
        Ok(new_frame)
    }
}

#[derive(Format)]
pub enum Frame {
    Standard(StandardFrame),
    Extended(ExtendedFrame),
}

impl Frame {
    pub fn data(&self) -> &[u8] {
        match self {
            Self::Standard(f) => f.data(),
            Self::Extended(f) => f.data(),
        }
    }
    pub fn length(&self) -> u8 {
        match self {
            Self::Standard(f) => f.length(),
            Self::Extended(f) => f.length(),
        }
    }
    pub fn src_addr(&self) -> IndividualAddress {
        match self {
            Self::Standard(f) => f.src_addr(),
            Self::Extended(f) => f.src_addr(),
        }
    }
    pub fn dst_addr(&self) -> Address {
        match self {
            Self::Standard(f) => f.dst_addr(),
            Self::Extended(f) => f.dst_addr(),
        }
    }
    pub fn hop_count(&self) -> u8 {
        match self {
            Self::Standard(f) => f.hop_count(),
            Self::Extended(f) => f.hop_count(),
        }
    }
    pub fn priority(&self) -> Priority {
        match self {
            Self::Standard(f) => f.priority(),
            Self::Extended(f) => f.priority(),
        }
    }
    pub fn repeated(&self) -> Repeated {
        match self {
            Self::Standard(f) => f.repeated(),
            Self::Extended(f) => f.repeated(),
        }
    }
    pub fn tpci(&self, bits: TpciBits) -> u8 {
        match self {
            Self::Standard(f) => f.tpci(bits),
            Self::Extended(f) => f.tpci(bits),
        }
    }
    pub fn tpci_seq(&self) -> u8 {
        match self {
            Self::Standard(f) => f.tpci_seq(),
            Self::Extended(f) => f.tpci_seq(),
        }
    }
    pub fn apci(&self, bits: ApciBits) -> u16 {
        match self {
            Self::Standard(f) => f.apci(bits),
            Self::Extended(f) => f.apci(bits),
        }
    }
    pub fn apdu_data(&self) -> &[u8] {
        match self {
            Self::Standard(f) => f.apdu_data(),
            Self::Extended(f) => f.apdu_data(),
        }
    }
    pub fn checksum(&self) -> u8 {
        match self {
            Self::Standard(f) => f.checksum(),
            Self::Extended(f) => f.checksum(),
        }
    }
    pub fn mut_data(&mut self) -> &mut [u8] {
        match self {
            Self::Standard(f) => f.mut_data(),
            Self::Extended(f) => f.mut_data(),
        }
    }
    pub fn set_repeated(&mut self, repeated: Repeated) {
        match self {
            Self::Standard(f) => f.set_repeated(repeated),
            Self::Extended(f) => f.set_repeated(repeated),
        }
    }
    pub fn set_src_addr(&mut self, src_addr: &IndividualAddress) {
        match self {
            Self::Standard(f) => f.set_src_addr(src_addr),
            Self::Extended(f) => f.set_src_addr(src_addr),
        }
    }
    pub fn set_dst_addr(&mut self, dst_addr: &Address) {
        match self {
            Self::Standard(f) => f.set_dst_addr(dst_addr),
            Self::Extended(f) => f.set_dst_addr(dst_addr),
        }
    }
    pub fn set_priority(&mut self, priority: Priority) {
        match self {
            Self::Standard(f) => f.set_priority(priority),
            Self::Extended(f) => f.set_priority(priority),
        }
    }
    pub fn set_hop_count(&mut self, hop_count: u8) {
        match self {
            Self::Standard(f) => f.set_hop_count(hop_count),
            Self::Extended(f) => f.set_hop_count(hop_count),
        }
    }
    pub fn set_tpci(&mut self, bits: TpciBits, val: u8) {
        match self {
            Self::Standard(f) => f.set_tpci(bits, val),
            Self::Extended(f) => f.set_tpci(bits, val),
        }
    }
    pub fn set_tpci_seq(&mut self, val: u8) {
        match self {
            Self::Standard(f) => f.set_tpci_seq(val),
            Self::Extended(f) => f.set_tpci_seq(val),
        }
    }
    pub fn set_apci(&mut self, bits: ApciBits, val: u16) {
        match self {
            Self::Standard(f) => f.set_apci(bits, val),
            Self::Extended(f) => f.set_apci(bits, val),
        }
    }
    pub fn from_datapoint(datapoint: &DataPoint) -> FrameResult<Self> {
        if datapoint.byte_length() > 14 {
            ExtendedFrame::from_datapoint(datapoint).map(|v| v.into())
        } else {
            StandardFrame::from_datapoint(datapoint).map(|v| v.into())
        }
    }
}

impl From<StandardFrame> for Frame {
    fn from(value: StandardFrame) -> Self {
        Self::Standard(value)
    }
}

impl From<ExtendedFrame> for Frame {
    fn from(value: ExtendedFrame) -> Self {
        Self::Extended(value)
    }
}

impl TryFrom<&Frame> for Frame {
    type Error = FrameError;
    fn try_from(value: &Frame) -> Result<Self, Self::Error> {
        match value {
            Frame::Standard(f) => StandardFrame::try_from(f).map(|f| f.into()),
            Frame::Extended(f) => ExtendedFrame::try_from(f).map(|f| f.into()),
        }
    }
}
