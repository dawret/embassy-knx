use crate::data_point::{DataPoint, DataPointAccess, DataPointLength};
use heapless::{
    pool::boxed::{Box, BoxBlock},
    Vec,
};
use num_enum::{IntoPrimitive, UnsafeFromPrimitive};

pub type SmallFrameBuf = Vec<u8, { FrameType::Standard.max_length() }, u8>;
pub type LargeFrameBuf = Vec<u8, { FrameType::Extended.max_length() }, u16>;
pub type SmallFrameBlock = BoxBlock<SmallFrameBuf>;
pub type LargeFrameBlock = BoxBlock<LargeFrameBuf>;
mod pool {
    #![allow(non_camel_case_types)]
    heapless::box_pool!(SMALL_POOL: super::SmallFrameBuf);
    heapless::box_pool!(LARGE_POOL: super::LargeFrameBuf);
}
use pool::{LARGE_POOL, SMALL_POOL};

/// Call once at startup, before any frame is allocated. Frames of up to 23 octets, which
/// includes every standard frame, take a small block; longer extended frames a large one.
pub fn init_frame_pool(
    small: &'static mut [SmallFrameBlock],
    large: &'static mut [LargeFrameBlock],
) {
    for block in small {
        SMALL_POOL.manage(block);
    }
    for block in large {
        LARGE_POOL.manage(block);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, UnsafeFromPrimitive, IntoPrimitive)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum FrameType {
    Standard = 1,
    Extended = 0,
}

impl FrameType {
    pub fn of(ctrl: u8) -> Self {
        // This is safe, possible values: 0, 1
        unsafe { FrameType::unchecked_transmute_from(ctrl >> 7) }
    }
    /// Up to and including the octet shared by TPCI and APCI.
    pub const fn header_length(self) -> usize {
        match self {
            FrameType::Standard => 7,
            FrameType::Extended => 8,
        }
    }
    /// Header and checksum, no APDU.
    pub const fn min_length(self) -> usize {
        self.header_length() + 1
    }
    pub const fn max_length(self) -> usize {
        match self {
            FrameType::Standard => 23,
            FrameType::Extended => 263,
        }
    }
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

enum Buf {
    Small(Box<SMALL_POOL>),
    Large(Box<LARGE_POOL>),
}

/// A telegram in one pool block. The field offsets of extended frames are shifted by their
/// second control octet.
pub struct Frame(Buf);

impl Frame {
    const CTRL: usize = 0;

    /// A zeroed frame of `length` octets including the checksum slot, not marked as repeated.
    pub fn new(frame_type: FrameType, length: usize) -> FrameResult<Self> {
        if length < frame_type.min_length() || length > frame_type.max_length() {
            return Err(FrameError::InvalidLength);
        }
        let mut frame = Self::alloc(length)?;
        let ctrl = match frame_type {
            FrameType::Standard => 0xB0,
            FrameType::Extended => 0x30,
        };
        frame.set_bits(Self::CTRL, 0xF3, ctrl);
        frame.set_length(length)?;
        Ok(frame)
    }

    fn alloc(length: usize) -> FrameResult<Self> {
        let buf = if length <= FrameType::Standard.max_length() {
            let mut buf = SMALL_POOL
                .alloc(Vec::new())
                .map_err(|_| FrameError::OutOfMemory)?;
            buf.resize_default(length)
                .map_err(|_| FrameError::InvalidLength)?;
            Buf::Small(buf)
        } else {
            let mut buf = LARGE_POOL
                .alloc(Vec::new())
                .map_err(|_| FrameError::OutOfMemory)?;
            buf.resize_default(length)
                .map_err(|_| FrameError::InvalidLength)?;
            Buf::Large(buf)
        };
        Ok(Frame(buf))
    }

    /// `bytes` includes the checksum octet.
    pub fn from_slice(bytes: &[u8]) -> FrameResult<Self> {
        let ctrl = *bytes.first().ok_or(FrameError::InvalidLength)?;
        let mut frame = Self::new(FrameType::of(ctrl), bytes.len())?;
        frame.mut_data().copy_from_slice(bytes);
        Ok(frame)
    }

    pub fn from_datapoint(datapoint: &DataPoint) -> FrameResult<Self> {
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
        let frame_type = if datapoint.byte_length() > 14 {
            FrameType::Extended
        } else {
            FrameType::Standard
        };
        let mut frame = Self::new(frame_type, frame_type.header_length() + 1 + size + 1)?;
        // Short values share the octet with the low APCI bits, longer ones follow it.
        let offset = frame.apci_offset() + 1;
        let (offset, octets) = match size {
            0 => (offset, 1),
            n => (offset + 1, n),
        };
        datapoint.write(&mut frame.mut_data()[offset..offset + octets]);
        Ok(frame)
    }

    /// Frame length including the checksum, from the first `header_length()` octets.
    pub fn length_in_header(header: &[u8]) -> usize {
        match FrameType::of(header[0]) {
            FrameType::Standard => (header[5] & 0xF) as usize + FrameType::Standard.min_length(),
            FrameType::Extended => header[6] as usize + FrameType::Extended.min_length(),
        }
    }

    pub fn data(&self) -> &[u8] {
        match &self.0 {
            Buf::Small(buf) => buf,
            Buf::Large(buf) => buf,
        }
    }

    pub fn mut_data(&mut self) -> &mut [u8] {
        match &mut self.0 {
            Buf::Small(buf) => buf,
            Buf::Large(buf) => buf,
        }
    }

    pub fn length(&self) -> usize {
        Self::length_in_header(self.data())
    }

    pub fn set_length(&mut self, length: usize) -> FrameResult<()> {
        let frame_type = self.frame_type();
        if length < frame_type.min_length() || length > frame_type.max_length() {
            return Err(FrameError::InvalidLength);
        }
        match &mut self.0 {
            Buf::Small(buf) => buf.resize_default(length),
            Buf::Large(buf) => buf.resize_default(length),
        }
        .map_err(|_| FrameError::InvalidLength)?;
        let lg = (length - frame_type.min_length()) as u8;
        match frame_type {
            FrameType::Standard => self.set_bits(5, 0x0F, lg),
            FrameType::Extended => self.mut_data()[6] = lg,
        }
        Ok(())
    }

    fn ext(&self) -> usize {
        (self.frame_type() == FrameType::Extended) as usize
    }
    fn src_addr_offset(&self) -> usize {
        1 + self.ext()
    }
    fn dst_addr_offset(&self) -> usize {
        3 + self.ext()
    }
    /// Address type and hop count.
    fn npci_offset(&self) -> usize {
        match self.frame_type() {
            FrameType::Standard => 5,
            FrameType::Extended => 1,
        }
    }
    fn tpci_offset(&self) -> usize {
        6 + self.ext()
    }
    fn apci_offset(&self) -> usize {
        self.tpci_offset()
    }

    pub fn frame_type(&self) -> FrameType {
        FrameType::of(self.data()[Self::CTRL])
    }
    pub fn addr_type(&self) -> AddressType {
        // This is safe, possible values: 0, 1
        unsafe { AddressType::unchecked_transmute_from(self.data()[self.npci_offset()] >> 7) }
    }
    pub fn src_addr(&self) -> IndividualAddress {
        IndividualAddress::from(&self.data()[self.src_addr_offset()..])
    }
    pub fn dst_addr(&self) -> Address {
        let dst_addr = &self.data()[self.dst_addr_offset()..];
        match self.addr_type() {
            AddressType::Group => Address::Group(GroupAddress::from(dst_addr)),
            AddressType::Individual => Address::Individual(IndividualAddress::from(dst_addr)),
        }
    }
    pub fn hop_count(&self) -> u8 {
        (self.data()[self.npci_offset()] >> 4) & 0x7
    }
    pub fn priority(&self) -> Priority {
        // This is safe, possible values: 0, 1, 2, 3
        unsafe { Priority::unchecked_transmute_from((self.data()[Self::CTRL] >> 2) & 0x3) }
    }
    pub fn repeated(&self) -> Repeated {
        // This is safe, possible values: 0, 1
        unsafe { Repeated::unchecked_transmute_from((self.data()[Self::CTRL] >> 5) & 0x1) }
    }
    pub fn tpci(&self, bits: TpciBits) -> u8 {
        match bits {
            TpciBits::Six => self.data()[self.tpci_offset()] >> 2,
            TpciBits::Eight => self.data()[self.tpci_offset()],
        }
    }
    pub fn tpci_seq(&self) -> u8 {
        (self.data()[self.tpci_offset()] >> 2) & 0xF
    }
    pub fn apci(&self, bits: ApciBits) -> u16 {
        let offset = self.apci_offset();
        let apci: u16 = ((self.data()[offset] as u16 & 0x3) << 8) | self.data()[offset + 1] as u16;
        match bits {
            ApciBits::Four => apci >> 6,
            ApciBits::Ten => apci,
        }
    }
    pub fn apdu_data(&self) -> &[u8] {
        &self.data()[self.apci_offset() + 1..self.length()]
    }
    /// Over all octets but the last, which holds the checksum.
    pub fn checksum(&self) -> u8 {
        let data = self.data();
        !data[..data.len() - 1].iter().fold(0, |acc, &v| acc ^ v)
    }

    fn set_bits(&mut self, index: usize, mask: u8, value: u8) {
        let octet = &mut self.mut_data()[index];
        *octet = (*octet & !mask) | (value & mask);
    }
    pub fn set_priority(&mut self, priority: Priority) {
        let p: u8 = priority.into();
        self.set_bits(Self::CTRL, 0x0C, p << 2);
    }
    pub fn set_repeated(&mut self, repeated: Repeated) {
        let r: u8 = repeated.into();
        self.set_bits(Self::CTRL, 0x20, r << 5);
    }
    pub fn set_hop_count(&mut self, hop_count: u8) {
        self.set_bits(self.npci_offset(), 0x70, hop_count << 4);
    }
    pub fn set_src_addr(&mut self, src_addr: &IndividualAddress) {
        let offset = self.src_addr_offset();
        src_addr.write(&mut self.mut_data()[offset..]);
    }
    pub fn set_dst_addr(&mut self, dst_addr: &Address) {
        let at = match dst_addr {
            Address::Individual(_) => AddressType::Individual,
            Address::Group(_) => AddressType::Group,
        };
        self.set_bits(self.npci_offset(), 0x80, u8::from(at) << 7);
        let offset = self.dst_addr_offset();
        dst_addr.write(&mut self.mut_data()[offset..]);
    }
    pub fn set_tpci(&mut self, bits: TpciBits, val: u8) {
        let offset = self.tpci_offset();
        match bits {
            // The low two bits of the TPCI octet belong to the APCI.
            TpciBits::Six => self.set_bits(offset, 0xFC, val << 2),
            TpciBits::Eight => self.mut_data()[offset] = val,
        }
    }
    pub fn set_tpci_seq(&mut self, val: u8) {
        self.set_bits(self.tpci_offset(), 0x3C, val << 2);
    }
    pub fn set_apci(&mut self, bits: ApciBits, val: u16) {
        let offset = self.apci_offset();
        match bits {
            ApciBits::Four => {
                self.set_bits(offset, 0x03, (val >> 2) as u8);
                self.set_bits(offset + 1, 0xC0, (val << 6) as u8);
            }
            ApciBits::Ten => {
                self.set_bits(offset, 0x03, (val >> 8) as u8);
                self.mut_data()[offset + 1] = val as u8;
            }
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Frame {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(fmt, "Frame type: {}\n", self.frame_type());
        defmt::write!(fmt, "Priority: {}\n", self.priority());
        defmt::write!(fmt, "Source address: {}\n", self.src_addr());
        defmt::write!(fmt, "Destination address: {}\n", self.dst_addr());
        defmt::write!(fmt, "Frame length: {}\n", self.length());
        defmt::write!(fmt, "Data: {:#04X}", self.data());
    }
}

impl core::fmt::Debug for Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}Frame({:02X?})", self.frame_type(), self.data())
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
        assert_eq!(f.frame_type(), FrameType::Standard);
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
        let mut f = Frame::new(FrameType::Standard, 9).unwrap();
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
        for frame_type in [FrameType::Standard, FrameType::Extended] {
            let f = Frame::new(frame_type, frame_type.min_length()).unwrap();
            assert_eq!(f.frame_type(), frame_type);
            assert_eq!(f.repeated(), Repeated::NotRepeated);
        }
    }

    #[test]
    fn setting_a_field_twice_overwrites_it() {
        init_pool();
        let mut f = Frame::new(FrameType::Standard, 9).unwrap();
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
        let mut f = Frame::new(FrameType::Standard, 8).unwrap();
        f.set_tpci(TpciBits::Six, 0x10);
        f.set_tpci_seq(12);
        assert_eq!(f.tpci_seq(), 12);
        assert_eq!(f.tpci(TpciBits::Eight), 0x70);
    }

    #[test]
    fn blocks_fit_their_frames() {
        assert!(core::mem::size_of::<SmallFrameBlock>() <= 32);
        assert!(core::mem::size_of::<LargeFrameBlock>() <= 272);
        assert!(core::mem::size_of::<Frame>() <= 2 * core::mem::size_of::<Box<SMALL_POOL>>());
    }

    #[test]
    fn extended_frame_fields_are_shifted() {
        init_pool();
        let mut f = Frame::new(FrameType::Extended, 12).unwrap();
        f.set_priority(Priority::Low);
        f.set_src_addr(&IndividualAddress::from_parts(1, 1, 10));
        f.set_dst_addr(&Address::Group(GroupAddress::from_parts(1, 1, 98)));
        f.set_hop_count(6);
        f.set_tpci(TpciBits::Six, 0);
        f.set_apci(ApciBits::Four, 2);
        assert_eq!(
            f.data(),
            [0x3C, 0xE0, 0x11, 0x0A, 0x09, 0x62, 0x03, 0x00, 0x80, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn standard_frame_length_limits() {
        init_pool();
        for frame_type in [FrameType::Standard, FrameType::Extended] {
            let (min, max) = (frame_type.min_length(), frame_type.max_length());
            assert_eq!(Frame::new(frame_type, min).unwrap().length(), min);
            assert_eq!(Frame::new(frame_type, max).unwrap().length(), max);
            assert_eq!(Frame::new(frame_type, max + 1).err(), Some(FrameError::InvalidLength));
            assert_eq!(Frame::new(frame_type, min - 1).err(), Some(FrameError::InvalidLength));
        }
    }

    #[test]
    fn parses_extended_frame() {
        let f = frame(&EXT_GROUP_VALUE_READ);
        assert_eq!(f.frame_type(), FrameType::Extended);
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
        let mut f = Frame::new(FrameType::Extended, 10).unwrap();
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
