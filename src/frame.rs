use crate::data_point::{DataPoint, DataPointAccess, DataPointLength};
use heapless::{
    pool::boxed::{Box, BoxBlock},
    Vec,
};
use num_enum::{IntoPrimitive, UnsafeFromPrimitive};

pub const MAX_FRAME_SIZE: usize = 264;
pub type FrameBuf = Vec<u8, MAX_FRAME_SIZE>;
pub type FrameBlock = BoxBlock<FrameBuf>;
mod pool {
    #![allow(non_camel_case_types)]
    heapless::box_pool!(FRAME_POOL: super::FrameBuf);
}
use pool::FRAME_POOL;

/// Call once at startup, before any frame is allocated.
pub fn init_frame_pool(blocks: &'static mut [FrameBlock]) {
    for block in blocks {
        FRAME_POOL.manage(block);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, UnsafeFromPrimitive, IntoPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum FrameType {
    Standard = 1,
    Extended = 0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FrameError {
    InvalidLength,
    OutOfMemory,
    Checksum,
    InvalidTpdu(u8),
}

type FrameResult<T> = Result<T, FrameError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, UnsafeFromPrimitive, IntoPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum AddressType {
    Individual = 0,
    Group = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, UnsafeFromPrimitive, IntoPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum Priority {
    Low = 0x3,
    Normal = 0x1,
    Urgent = 0x2,
    System = 0x0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, UnsafeFromPrimitive, IntoPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum Repeated {
    Repeated = 0,
    NotRepeated = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Hash)]
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

#[cfg(feature = "defmt")]
impl defmt::Format for IndividualAddress {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(
            fmt,
            "{}.{}.{}",
            self.0 >> 12,
            (self.0 >> 8) & 0xF,
            self.0 & 0xFF
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Hash)]
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

#[cfg(feature = "defmt")]
impl defmt::Format for GroupAddress {
    fn format(&self, fmt: defmt::Formatter) {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(usize)]
pub enum TpciBits {
    Six = 6,
    Eight = 8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    fn length(&self) -> usize;
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
        (self.data()[Self::TPCI_OFFSET] >> 2) & 0xF
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
        &self.data()[Self::APCI_OFFSET + 1..self.length()]
    }

    /// Over all octets but the last, which holds the checksum.
    fn checksum(&self) -> u8 {
        let data = self.data();
        !data[..data.len() - 1].iter().fold(0, |acc, &v| acc ^ v)
    }

    #[cfg(feature = "defmt")]
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
    fn set_bits(&mut self, index: usize, mask: u8, value: u8) {
        let octet = &mut self.mut_data()[index];
        *octet = (*octet & !mask) | (value & mask);
    }
    fn set_addr_type(&mut self, addr_type: AddressType) {
        let at: u8 = addr_type.into();
        self.set_bits(Self::AT_FIELD, 0x80, at << 7);
    }
    fn set_priority(&mut self, priority: Priority) {
        let p: u8 = priority.into();
        self.set_bits(Self::CTRL, 0x0C, p << 2);
    }
    fn set_repeated(&mut self, repeated: Repeated) {
        let r: u8 = repeated.into();
        self.set_bits(Self::CTRL, 0x20, r << 5);
    }
    fn set_hop_count(&mut self, hop_count: u8) {
        self.set_bits(Self::HOP_COUNT_FIELD, 0x70, hop_count << 4);
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
        // Short values share the octet with the low APCI bits, longer ones follow it.
        let (offset, octets) = match size {
            0 => (Self::APCI_OFFSET + 1, 1),
            n => (Self::APCI_OFFSET + 2, n),
        };
        datapoint.write(&mut frame.mut_data()[offset..offset + octets]);
        frame.set_length(Self::HEADER_LENGTH + Self::APCI_BASE_SIZE + size + 1)?;
        Ok(frame)
    }
    fn set_tpci(&mut self, bits: TpciBits, val: u8) {
        match bits {
            // The low two bits of the TPCI octet belong to the APCI.
            TpciBits::Six => self.set_bits(Self::TPCI_OFFSET, 0xFC, val << 2),
            TpciBits::Eight => self.mut_data()[Self::TPCI_OFFSET] = val,
        }
    }
    fn set_tpci_seq(&mut self, val: u8) {
        self.set_bits(Self::TPCI_OFFSET, 0x3C, val << 2);
    }
    fn set_apci(&mut self, bits: ApciBits, val: u16) {
        match bits {
            ApciBits::Four => {
                self.set_bits(Self::APCI_OFFSET, 0x03, (val >> 2) as u8);
                self.set_bits(Self::APCI_OFFSET + 1, 0xC0, (val << 6) as u8);
            }
            ApciBits::Ten => {
                self.set_bits(Self::APCI_OFFSET, 0x03, (val >> 8) as u8);
                self.mut_data()[Self::APCI_OFFSET + 1] = val as u8;
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
    fn length(&self) -> usize {
        (self.0[Self::LG_FIELD] & 0xF) as usize + Self::HEADER_LENGTH + 1
    }
}

impl FrameWriter for StandardFrame {
    fn mut_data(&mut self) -> &mut [u8] {
        &mut self.0
    }
    fn set_length(&mut self, length: usize) -> FrameResult<()> {
        if length <= Self::MAX_FRAME_SIZE && length >= Self::MIN_FRAME_SIZE {
            let lg = (length - Self::HEADER_LENGTH - 1) as u8;
            self.set_bits(Self::LG_FIELD, 0x0F, lg);
            self.0
                .resize_default(length)
                .map_err(|_| FrameError::InvalidLength)?;
            Ok(())
        } else {
            Err(FrameError::InvalidLength)
        }
    }
    fn set_frame_type(&mut self) {
        // Standard frame, not repeated. Keeps the priority.
        self.set_bits(Self::CTRL, 0xF3, 0xB0);
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

#[cfg(feature = "defmt")]
impl defmt::Format for StandardFrame {
    fn format(&self, fmt: defmt::Formatter) {
        FrameReader::format(self, fmt);
    }
}

pub struct ExtendedFrame(Box<FRAME_POOL>);

impl FrameReader for ExtendedFrame {
    const CTRL_OFFSET: usize = 2;
    const LG_FIELD: usize = 6;
    const AT_FIELD: usize = 1;
    const HOP_COUNT_FIELD: usize = 1;
    const MAX_FRAME_SIZE: usize = 263;
    const MIN_FRAME_SIZE: usize = 9;

    fn data(&self) -> &[u8] {
        &self.0
    }
    fn length(&self) -> usize {
        self.0[Self::LG_FIELD] as usize + Self::HEADER_LENGTH + 1
    }
}

impl FrameWriter for ExtendedFrame {
    fn mut_data(&mut self) -> &mut [u8] {
        &mut self.0
    }
    fn set_length(&mut self, length: usize) -> FrameResult<()> {
        if length <= Self::MAX_FRAME_SIZE && length >= Self::MIN_FRAME_SIZE {
            self.0[Self::LG_FIELD] = (length - Self::HEADER_LENGTH - 1) as u8;
            self.0
                .resize_default(length)
                .map_err(|_| FrameError::InvalidLength)?;
            Ok(())
        } else {
            Err(FrameError::InvalidLength)
        }
    }
    fn set_frame_type(&mut self) {
        // Extended frame, not repeated. Keeps the priority.
        self.set_bits(Self::CTRL, 0xF3, 0x30);
    }
    fn new(size: usize) -> FrameResult<Self> {
        let mut buf = FRAME_POOL
            .alloc(Vec::new())
            .map_err(|_| FrameError::OutOfMemory)?;
        buf.resize_default(size)
            .map_err(|_| FrameError::InvalidLength)?;
        let mut frame = ExtendedFrame(buf);
        frame.set_frame_type();
        Ok(frame)
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for ExtendedFrame {
    fn format(&self, fmt: defmt::Formatter) {
        FrameReader::format(self, fmt);
    }
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    pub fn length(&self) -> usize {
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
    /// `bytes` includes the checksum octet.
    pub fn from_slice(bytes: &[u8]) -> FrameResult<Self> {
        let mut frame: Frame = match bytes.first() {
            None => return Err(FrameError::InvalidLength),
            Some(ctrl) if ctrl >> 7 == FrameType::Standard as u8 => {
                StandardFrame::new(bytes.len())?.into()
            }
            Some(_) => ExtendedFrame::new(bytes.len())?.into(),
        };
        frame.mut_data().copy_from_slice(bytes);
        Ok(frame)
    }
}

impl core::fmt::Debug for Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let kind = match self {
            Self::Standard(_) => "Standard",
            Self::Extended(_) => "Extended",
        };
        write!(f, "{}Frame({:02X?})", kind, self.data())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_point::{B1, U16, U8};
    use crate::test_util::{frame, init_pool, with_checksum};

    const GROUP_VALUE_READ: [u8; 8] = [0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE1, 0x00, 0x00];

    const EXT_GROUP_VALUE_READ: [u8; 9] = [0x3C, 0xE0, 0x11, 0x0A, 0x09, 0x62, 0x01, 0x00, 0x00];

    #[test]
    fn addresses_from_parts() {
        assert_eq!(IndividualAddress::from_parts(1, 1, 120), IndividualAddress::new(0x1178));
        assert_eq!(IndividualAddress::from_parts(15, 15, 255), IndividualAddress::new(0xFFFF));
        assert_eq!(GroupAddress::from_parts(1, 1, 98), GroupAddress::new(0x0962));
        assert_eq!(GroupAddress::from_parts(31, 7, 255), GroupAddress::new(0xFFFF));
    }

    #[test]
    fn parses_standard_group_frame() {
        let f = frame(&GROUP_VALUE_READ);
        assert!(matches!(f, Frame::Standard(_)));
        assert_eq!(f.src_addr(), IndividualAddress::from_parts(1, 1, 10));
        assert_eq!(f.dst_addr(), Address::Group(GroupAddress::from_parts(1, 1, 98)));
        assert_eq!(f.priority(), Priority::Low);
        assert_eq!(f.repeated(), Repeated::NotRepeated);
        assert_eq!(f.hop_count(), 6);
        assert_eq!(f.length(), 9);
        assert_eq!(f.tpci(TpciBits::Six), 0);
        assert_eq!(f.apci(ApciBits::Four), 0);
    }

    #[test]
    fn parses_individual_frame() {
        let f = frame(&[0xB0, 0x11, 0xFA, 0x11, 0x78, 0x60, 0x80]);
        assert_eq!(f.dst_addr(), Address::Individual(IndividualAddress::from_parts(1, 1, 120)));
        assert_eq!(f.priority(), Priority::System);
        assert_eq!(f.length(), 8);
        assert_eq!(f.tpci(TpciBits::Eight), 0x80);
    }

    #[test]
    fn from_slice_rejects_empty_input() {
        assert_eq!(Frame::from_slice(&[]).unwrap_err(), FrameError::InvalidLength);
    }

    #[test]
    fn builds_group_frame_with_setters() {
        init_pool();
        let mut f = StandardFrame::new(9).unwrap();
        f.set_priority(Priority::Low);
        f.set_src_addr(&IndividualAddress::from_parts(1, 1, 10));
        f.set_dst_addr(&Address::Group(GroupAddress::from_parts(1, 1, 98)));
        f.set_hop_count(6);
        f.set_tpci(TpciBits::Six, 0);
        f.set_apci(ApciBits::Four, 0);
        // The last octet is the checksum slot, filled in by the driver.
        assert_eq!(f.data(), &[GROUP_VALUE_READ.as_slice(), &[0]].concat()[..]);
    }

    #[test]
    fn new_frames_are_not_marked_repeated() {
        init_pool();
        assert_eq!(Frame::from(StandardFrame::new(8).unwrap()).repeated(), Repeated::NotRepeated);
        assert_eq!(Frame::from(ExtendedFrame::new(9).unwrap()).repeated(), Repeated::NotRepeated);
    }

    #[test]
    fn setting_a_field_twice_overwrites_it() {
        init_pool();
        let mut f = StandardFrame::new(9).unwrap();
        f.set_priority(Priority::Low);
        f.set_priority(Priority::Normal);
        assert_eq!(f.priority(), Priority::Normal);

        f.set_hop_count(7);
        f.set_hop_count(5);
        assert_eq!(f.hop_count(), 5);

        f.set_dst_addr(&Address::Group(GroupAddress::from_parts(1, 1, 98)));
        f.set_dst_addr(&Address::Individual(IndividualAddress::from_parts(1, 1, 5)));
        assert_eq!(f.dst_addr(), Address::Individual(IndividualAddress::from_parts(1, 1, 5)));

        f.set_tpci(TpciBits::Six, 0x10);
        f.set_tpci_seq(5);
        f.set_tpci_seq(3);
        assert_eq!(f.tpci_seq(), 3);

        f.set_apci(ApciBits::Four, 0x1);
        f.set_apci(ApciBits::Four, 0x2);
        assert_eq!(f.apci(ApciBits::Four), 0x2);
    }

    #[test]
    fn sequence_number_has_four_bits() {
        let f = frame(&[0xB0, 0x11, 0xFA, 0x11, 0x78, 0x61, 0x7F, 0x00]);
        assert_eq!(f.tpci_seq(), 15);

        init_pool();
        let mut f = StandardFrame::new(8).unwrap();
        f.set_tpci(TpciBits::Six, 0x10);
        f.set_tpci_seq(12);
        assert_eq!(f.tpci_seq(), 12);
        assert_eq!(f.tpci(TpciBits::Eight), 0x70);
    }

    #[test]
    fn standard_frame_length_limits() {
        init_pool();
        assert!(StandardFrame::new(StandardFrame::MIN_FRAME_SIZE).is_ok());
        assert!(StandardFrame::new(StandardFrame::MAX_FRAME_SIZE).is_ok());
        assert_eq!(
            StandardFrame::new(StandardFrame::MAX_FRAME_SIZE + 1).err(),
            Some(FrameError::InvalidLength)
        );
        assert_eq!(
            StandardFrame::new(StandardFrame::MIN_FRAME_SIZE - 1).err(),
            Some(FrameError::InvalidLength)
        );
    }

    #[test]
    fn parses_extended_frame() {
        let f = frame(&EXT_GROUP_VALUE_READ);
        assert!(matches!(f, Frame::Extended(_)));
        assert_eq!(f.src_addr(), IndividualAddress::from_parts(1, 1, 10));
        assert_eq!(f.dst_addr(), Address::Group(GroupAddress::from_parts(1, 1, 98)));
        assert_eq!(f.hop_count(), 6);
        assert_eq!(f.length(), 10);
    }

    #[test]
    fn long_extended_frame_length() {
        let mut telegram = [0u8; 259 - 1];
        telegram[..7].copy_from_slice(&EXT_GROUP_VALUE_READ[..7]);
        telegram[6] = 250;
        assert_eq!(frame(&telegram).length(), 259);
    }

    #[test]
    fn extended_set_length_writes_requested_length() {
        init_pool();
        let mut f = ExtendedFrame::new(10).unwrap();
        f.set_length(12).unwrap();
        assert_eq!(f.data().len(), 12);
        assert_eq!(f.data()[6], 12 - 9);
        assert_eq!(f.length(), 12);
    }

    #[test]
    fn frame_from_short_datapoint() {
        init_pool();
        let f = Frame::from_datapoint(&DataPoint::B1(B1::new(true))).unwrap();
        assert_eq!(f.length(), 9);
        assert_eq!(f.data()[7] & 0x3F, 0x01);
    }

    #[test]
    fn frame_from_byte_datapoints() {
        init_pool();
        let f = Frame::from_datapoint(&DataPoint::U8(U8::new(0xA5))).unwrap();
        assert_eq!(f.length(), 10);
        assert_eq!(&f.data()[8..9], &[0xA5]);

        let f = Frame::from_datapoint(&DataPoint::U16(U16::new(0x1234))).unwrap();
        assert_eq!(f.length(), 11);
        assert_eq!(&f.data()[8..10], &[0x12, 0x34]);
    }

    #[test]
    fn debug_shows_bytes() {
        let f = frame(&GROUP_VALUE_READ);
        let expected = format!("StandardFrame({:02X?})", with_checksum(&GROUP_VALUE_READ));
        assert_eq!(format!("{f:?}"), expected);
    }
}
