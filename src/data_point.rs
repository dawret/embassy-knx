use defmt::*;
use enum_dispatch::enum_dispatch;

#[derive(Format)]
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
#[derive(Format)]
pub enum DataPoint {
    B1,
    B2,
    //B1U3,
    //Char,
    U8,
    /*V8,
    StatusMode,
    U16,
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

#[derive(Format)]
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

#[derive(Format)]
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
#[derive(Format)]
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
