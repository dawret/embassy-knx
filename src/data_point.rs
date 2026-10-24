use enum_dispatch::enum_dispatch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DataPointLength {
    Bit(usize),
    Byte(usize),
}

#[enum_dispatch]
pub trait DataPointAccess {
    fn length(&self) -> DataPointLength;
    fn write(&self, buf: &mut [u8]);
    fn byte_length(&self) -> usize {
        match self.length() {
            DataPointLength::Bit(n) if n < 6 => 0,
            DataPointLength::Bit(_) => 1,
            DataPointLength::Byte(n) => n,
        }
    }
}

#[enum_dispatch(DataPointAccess)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DataPoint {
    B1,
    B2,
    //B1U3,
    //Char,
    U8,
    U16,
    /*V8,
    StatusMode,
    S16,
    F16,
    Time,
    Date,
    U32,
    S32,
    F32,
    AccessData,
    String,
    SceneNumber,
    SceneControl,
    DateTime,
    N8,
    B8,
    N2,
    VarString,
    SceneInfo,
    B32,
    UnicodeString,
    V64,*/
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct B1(u8);

impl B1 {
    pub fn new(data: bool) -> Self {
        Self(data as u8)
    }
}

impl DataPointAccess for B1 {
    fn length(&self) -> DataPointLength {
        DataPointLength::Bit(1)
    }
    fn write(&self, buf: &mut [u8]) {
        buf[0] |= self.0 & 0x1;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct B2(u8);
impl B2 {
    pub fn new(data: u8) -> Self {
        Self(data)
    }
}

impl DataPointAccess for B2 {
    fn length(&self) -> DataPointLength {
        DataPointLength::Bit(2)
    }
    fn write(&self, buf: &mut [u8]) {
        buf[0] |= self.0 & 0x3;
    }
}
//pub struct B1U3 {}
//pub struct Char {}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct U8(u8);
impl U8 {
    pub fn new(data: u8) -> Self {
        Self(data)
    }
}
impl DataPointAccess for U8 {
    fn length(&self) -> DataPointLength {
        DataPointLength::Byte(1)
    }
    fn write(&self, buf: &mut [u8]) {
        buf[0] = self.0;
    }
}
//pub struct V8 {}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct U16(u16);
impl U16 {
    pub fn new(data: u16) -> Self {
        Self(data)
    }
}
impl DataPointAccess for U16 {
    fn length(&self) -> DataPointLength {
        DataPointLength::Byte(2)
    }
    fn write(&self, buf: &mut [u8]) {
        buf[..2].copy_from_slice(&self.0.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        let cases = [
            (DataPoint::B1(B1::new(true)), DataPointLength::Bit(1), 0),
            (DataPoint::B2(B2::new(2)), DataPointLength::Bit(2), 0),
            (DataPoint::U8(U8::new(7)), DataPointLength::Byte(1), 1),
            (DataPoint::U16(U16::new(7)), DataPointLength::Byte(2), 2),
        ];
        for (dp, length, byte_length) in cases {
            assert_eq!(dp.length(), length, "{dp:?}");
            assert_eq!(dp.byte_length(), byte_length, "{dp:?}");
        }
    }

    #[test]
    fn short_values_keep_the_apci_bits() {
        // The two high bits belong to the APCI (here A_GroupValue_Write).
        let mut buf = [0x80];
        B1::new(true).write(&mut buf);
        assert_eq!(buf, [0x81]);

        let mut buf = [0x80];
        B2::new(0x3).write(&mut buf);
        assert_eq!(buf, [0x83]);
    }

    #[test]
    fn byte_values_are_big_endian() {
        let mut buf = [0; 1];
        U8::new(0xA5).write(&mut buf);
        assert_eq!(buf, [0xA5]);

        let mut buf = [0; 2];
        U16::new(0x1234).write(&mut buf);
        assert_eq!(buf, [0x12, 0x34]);
    }
}
