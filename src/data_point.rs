use crate::group_object::ValueType;

/// A datapoint type: the encoding of a group object value. `buf` is the object's value
/// storage, `VALUE_TYPE.size_octets()` long; values of up to 7 bits are right-aligned.
pub trait Dpt: Sized {
    const VALUE_TYPE: ValueType;
    fn encode(&self, buf: &mut [u8]);
    fn decode(buf: &[u8]) -> Self;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct B1(bool);

impl B1 {
    pub fn new(data: bool) -> Self {
        Self(data)
    }
    pub fn value(self) -> bool {
        self.0
    }
}

impl Dpt for B1 {
    const VALUE_TYPE: ValueType = ValueType::bits(1);
    fn encode(&self, buf: &mut [u8]) {
        buf[0] = self.0 as u8;
    }
    fn decode(buf: &[u8]) -> Self {
        Self(buf[0] & 0x1 != 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct B2(u8);

impl B2 {
    pub fn new(data: u8) -> Self {
        Self(data & 0x3)
    }
    pub fn value(self) -> u8 {
        self.0
    }
}

impl Dpt for B2 {
    const VALUE_TYPE: ValueType = ValueType::bits(2);
    fn encode(&self, buf: &mut [u8]) {
        buf[0] = self.0;
    }
    fn decode(buf: &[u8]) -> Self {
        Self(buf[0] & 0x3)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct U8(u8);

impl U8 {
    pub fn new(data: u8) -> Self {
        Self(data)
    }
    pub fn value(self) -> u8 {
        self.0
    }
}

impl Dpt for U8 {
    const VALUE_TYPE: ValueType = ValueType::octets(1);
    fn encode(&self, buf: &mut [u8]) {
        buf[0] = self.0;
    }
    fn decode(buf: &[u8]) -> Self {
        Self(buf[0])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct U16(u16);

impl U16 {
    pub fn new(data: u16) -> Self {
        Self(data)
    }
    pub fn value(self) -> u16 {
        self.0
    }
}

impl Dpt for U16 {
    const VALUE_TYPE: ValueType = ValueType::octets(2);
    fn encode(&self, buf: &mut [u8]) {
        buf[..2].copy_from_slice(&self.0.to_be_bytes());
    }
    fn decode(buf: &[u8]) -> Self {
        Self(u16::from_be_bytes([buf[0], buf[1]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Dpt + PartialEq + core::fmt::Debug>(value: T, encoded: &[u8]) {
        let mut buf = vec![0xFF; T::VALUE_TYPE.size_octets()];
        value.encode(&mut buf);
        assert_eq!(buf, encoded, "{value:?}");
        assert_eq!(T::decode(&buf), value);
    }

    #[test]
    fn values_round_trip() {
        round_trip(B1::new(true), &[0x01]);
        round_trip(B1::new(false), &[0x00]);
        round_trip(B2::new(0x3), &[0x03]);
        round_trip(U8::new(0xA5), &[0xA5]);
        round_trip(U16::new(0x1234), &[0x12, 0x34]);
    }

    #[test]
    fn value_types() {
        assert!(B1::VALUE_TYPE.is_short());
        assert_eq!(B2::VALUE_TYPE.size_bits(), 2);
        assert!(!U8::VALUE_TYPE.is_short());
        assert_eq!(U16::VALUE_TYPE.size_octets(), 2);
    }
}
