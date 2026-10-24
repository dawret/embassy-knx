use crate::bus::ConStatus;
use crate::data_point::Dpt;
use crate::frame::{Frame, GroupService, Priority};
use core::cell::RefCell;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::signal::Signal;
use num_enum::UnsafeFromPrimitive;

/// Group object number, starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Asap(u16);

impl Asap {
    pub const fn new(number: u16) -> Self {
        assert!(number > 0);
        Self(number)
    }
    pub const fn get(self) -> u16 {
        self.0
    }
    fn index(self) -> usize {
        self.0 as usize - 1
    }
}

/// Size of a group object value, as the "Value Field Type" codes of the System B group object
/// descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ValueType(u8);

impl ValueType {
    pub const fn from_code(code: u8) -> Self {
        Self(code)
    }
    pub const fn bits(n: u8) -> Self {
        assert!(n >= 1 && n <= 7);
        Self(n - 1)
    }
    pub const fn octets(n: usize) -> Self {
        let mut code = 7;
        loop {
            if Self(code).size_octets() == n {
                return Self(code);
            }
            assert!(code < 255, "no value type of this size");
            code += 1;
        }
    }
    pub const fn code(self) -> u8 {
        self.0
    }
    pub const fn size_bits(self) -> usize {
        match self.0 {
            0..=6 => self.0 as usize + 1,
            _ => self.size_octets() * 8,
        }
    }
    /// Octets in the group object storage; values of up to 7 bits take one.
    pub const fn size_octets(self) -> usize {
        match self.0 {
            0..=7 => 1,
            8..=10 => self.0 as usize - 6,
            11 => 6,
            12 => 8,
            13 => 10,
            14 => 14,
            15 => 5,
            16 => 7,
            17 => 9,
            18 => 11,
            19 => 12,
            20 => 13,
            21..=254 => self.0 as usize - 6,
            255 => 252,
        }
    }
    /// Sent in the low 6 bits of the APCI octet.
    pub const fn is_short(self) -> bool {
        self.0 <= 5
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ConfigFlag {
    Communication = 1 << 10,
    Read = 1 << 11,
    Write = 1 << 12,
    ReadOnInit = 1 << 13,
    Transmit = 1 << 14,
    Update = 1 << 15,
}

/// The System B group object descriptor: config flags, priority and value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Descriptor(u16);

impl Descriptor {
    pub const fn new(value_type: ValueType, priority: Priority, flags: &[ConfigFlag]) -> Self {
        let mut bits = (priority as u16) << 8 | value_type.0 as u16;
        let mut i = 0;
        while i < flags.len() {
            bits |= flags[i] as u16;
            i += 1;
        }
        Self(bits)
    }
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }
    pub const fn bits(self) -> u16 {
        self.0
    }
    pub const fn value_type(self) -> ValueType {
        ValueType(self.0 as u8)
    }
    pub fn priority(self) -> Priority {
        // This is safe, possible values: 0, 1, 2, 3
        unsafe { Priority::unchecked_transmute_from((self.0 >> 8) as u8 & 0x3) }
    }
    pub const fn has(self, flag: ConfigFlag) -> bool {
        self.0 & flag as u16 != 0
    }
    fn enabled(self, flag: ConfigFlag) -> bool {
        self.has(ConfigFlag::Communication) && self.has(flag)
    }
}

/// Octets of value storage for `descriptors`.
pub const fn value_bytes(descriptors: &[Descriptor]) -> usize {
    let mut bytes = 0;
    let mut i = 0;
    while i < descriptors.len() {
        bytes += descriptors[i].value_type().size_octets();
        i += 1;
    }
    bytes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TransmissionStatus {
    IdleOk = 0,
    IdleError = 1,
    Transmitting = 2,
    TransmitRequest = 3,
}

/// Value offset and communication flags of one group object, encoded as in the RAM flags table.
#[derive(Debug, Clone, Copy)]
pub struct GroupObjectRam {
    offset: u16,
    flags: u8,
}

impl GroupObjectRam {
    const UPDATE: u8 = 0x08;
    const DATA_REQUEST: u8 = 0x04;
    const STATUS: u8 = 0x03;

    pub const fn new() -> Self {
        Self { offset: 0, flags: 0 }
    }
    fn status(&self) -> TransmissionStatus {
        match self.flags & Self::STATUS {
            0 => TransmissionStatus::IdleOk,
            1 => TransmissionStatus::IdleError,
            2 => TransmissionStatus::Transmitting,
            _ => TransmissionStatus::TransmitRequest,
        }
    }
    fn set_status(&mut self, status: TransmissionStatus) {
        self.flags = (self.flags & !Self::STATUS) | status as u8;
    }
    fn is(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
    fn set(&mut self, flag: u8, on: bool) {
        if on {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }
}

impl Default for GroupObjectRam {
    fn default() -> Self {
        Self::new()
    }
}

struct State {
    values: &'static mut [u8],
    ram: &'static mut [GroupObjectRam],
}

/// The group object table shared by the stack and the application. The stack answers reads
/// and stores writes on its own; the two sides only exchange the communication flags.
pub struct GroupObjects {
    descriptors: &'static [Descriptor],
    state: Mutex<CriticalSectionRawMutex, RefCell<State>>,
    transmit_requested: Signal<CriticalSectionRawMutex, ()>,
    updated: Signal<CriticalSectionRawMutex, ()>,
}

impl GroupObjects {
    /// `values` must hold at least `value_bytes(descriptors)` octets, `ram` one entry per
    /// descriptor.
    pub fn new(
        descriptors: &'static [Descriptor],
        values: &'static mut [u8],
        ram: &'static mut [GroupObjectRam],
    ) -> Self {
        assert!(values.len() >= value_bytes(descriptors));
        assert!(ram.len() == descriptors.len());
        let mut offset = 0;
        for (ram, descriptor) in ram.iter_mut().zip(descriptors) {
            *ram = GroupObjectRam {
                offset: offset as u16,
                flags: 0,
            };
            offset += descriptor.value_type().size_octets();
        }
        Self {
            descriptors,
            state: Mutex::new(RefCell::new(State { values, ram })),
            transmit_requested: Signal::new(),
            updated: Signal::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    pub fn descriptor(&self, asap: Asap) -> Option<Descriptor> {
        self.descriptors.get(asap.index()).copied()
    }

    fn with<R>(
        &self,
        asap: Asap,
        f: impl FnOnce(Descriptor, &mut GroupObjectRam, &mut [u8]) -> R,
    ) -> Option<R> {
        let descriptor = self.descriptor(asap)?;
        Some(self.state.lock(|state| {
            let state = &mut *state.borrow_mut();
            let ram = &mut state.ram[asap.index()];
            let offset = ram.offset as usize;
            let size = descriptor.value_type().size_octets();
            f(descriptor, ram, &mut state.values[offset..offset + size])
        }))
    }

    fn with_typed<T: Dpt, R>(
        &self,
        asap: Asap,
        f: impl FnOnce(&mut GroupObjectRam, &mut [u8]) -> R,
    ) -> R {
        unwrap!(self.with(asap, |descriptor, ram, value| {
            assert!(descriptor.value_type() == T::VALUE_TYPE);
            f(ram, value)
        }))
    }

    pub fn get<T: Dpt>(&self, asap: Asap) -> T {
        self.with_typed::<T, _>(asap, |_, value| T::decode(value))
    }

    /// Stores the value without sending it; bus reads are answered with it.
    pub fn store<T: Dpt>(&self, asap: Asap, value: &T) {
        self.with_typed::<T, _>(asap, |_, buf| value.encode(buf));
    }

    /// Stores the value and sends it with A_GroupValue_Write.
    pub fn set<T: Dpt>(&self, asap: Asap, value: &T) {
        self.with_typed::<T, _>(asap, |ram, buf| {
            value.encode(buf);
            ram.set(GroupObjectRam::DATA_REQUEST, false);
            ram.set_status(TransmissionStatus::TransmitRequest);
        });
        self.transmit_requested.signal(());
    }

    /// Sends A_GroupValue_Read; the response arrives through `updated()`.
    pub fn request_read(&self, asap: Asap) {
        unwrap!(self.with(asap, |_, ram, _| {
            ram.set(GroupObjectRam::DATA_REQUEST, true);
            ram.set_status(TransmissionStatus::TransmitRequest);
        }));
        self.transmit_requested.signal(());
    }

    pub fn status(&self, asap: Asap) -> TransmissionStatus {
        unwrap!(self.with(asap, |_, ram, _| ram.status()))
    }

    /// The next group object whose value was updated from the bus. Clears its update flag.
    pub async fn updated(&self) -> Asap {
        loop {
            let next = self.state.lock(|state| {
                let state = &mut *state.borrow_mut();
                let (index, ram) = state
                    .ram
                    .iter_mut()
                    .enumerate()
                    .find(|(_, ram)| ram.is(GroupObjectRam::UPDATE))?;
                ram.set(GroupObjectRam::UPDATE, false);
                Some(Asap(index as u16 + 1))
            });
            if let Some(asap) = next {
                return asap;
            }
            self.updated.wait().await;
        }
    }

    /// A Write or Response from the bus. Returns whether the value was stored.
    pub(crate) fn bus_value(&self, asap: Asap, service: GroupService, frame: &Frame) -> bool {
        let stored = self
            .with(asap, |descriptor, ram, value| {
                let flag = match service {
                    GroupService::Write => ConfigFlag::Write,
                    GroupService::Response => ConfigFlag::Update,
                    GroupService::Read => return false,
                };
                let value_type = descriptor.value_type();
                if !descriptor.enabled(flag) {
                    return false;
                }
                if value_type.is_short() {
                    if !frame.apdu_data().is_empty() {
                        return false;
                    }
                    value[0] = frame.short_data() & ((1 << value_type.size_bits()) - 1);
                } else if frame.apdu_data().len() == value.len() {
                    value.copy_from_slice(frame.apdu_data());
                } else {
                    return false;
                }
                ram.set(GroupObjectRam::UPDATE, true);
                true
            })
            .unwrap_or(false);
        if stored {
            self.updated.signal(());
        }
        stored
    }

    /// The A_GroupValue_Response to a bus read, if the object answers reads.
    pub(crate) fn response_frame(&self, asap: Asap) -> Option<Frame> {
        self.with(asap, |descriptor, _, value| {
            if !descriptor.enabled(ConfigFlag::Read) {
                return None;
            }
            let frame = Frame::group_value(
                GroupService::Response,
                descriptor.priority(),
                descriptor.value_type(),
                value,
            );
            match frame {
                Ok(frame) => Some(frame),
                Err(e) => {
                    error!("Group value response: {}", e);
                    None
                }
            }
        })
        .flatten()
    }

    pub(crate) async fn transmit_requested(&self) {
        self.transmit_requested.wait().await
    }

    pub(crate) fn signal_transmit_request(&self) {
        self.transmit_requested.signal(());
    }

    pub(crate) fn transmit_pending(&self) -> bool {
        self.state.lock(|state| {
            state
                .borrow()
                .ram
                .iter()
                .any(|ram| ram.status() == TransmissionStatus::TransmitRequest)
        })
    }

    /// Takes the next transmit request and builds its frame. Requests on objects without the
    /// C and T flags end in `IdleError` without a frame.
    pub(crate) fn start_transmission(&self) -> Option<(Asap, Frame)> {
        self.state.lock(|state| {
            let state = &mut *state.borrow_mut();
            for (index, (ram, descriptor)) in
                state.ram.iter_mut().zip(self.descriptors).enumerate()
            {
                if ram.status() != TransmissionStatus::TransmitRequest {
                    continue;
                }
                if !descriptor.enabled(ConfigFlag::Transmit) {
                    ram.set_status(TransmissionStatus::IdleError);
                    continue;
                }
                let frame = if ram.is(GroupObjectRam::DATA_REQUEST) {
                    Frame::group_read(descriptor.priority())
                } else {
                    let offset = ram.offset as usize;
                    let size = descriptor.value_type().size_octets();
                    Frame::group_value(
                        GroupService::Write,
                        descriptor.priority(),
                        descriptor.value_type(),
                        &state.values[offset..offset + size],
                    )
                };
                match frame {
                    Ok(frame) => {
                        ram.set_status(TransmissionStatus::Transmitting);
                        return Some((Asap(index as u16 + 1), frame));
                    }
                    Err(e) => {
                        error!("Group value transmission: {}", e);
                        ram.set_status(TransmissionStatus::IdleError);
                    }
                }
            }
            None
        })
    }

    /// L_Data.con of a transmission. A request made while it was in progress stays pending.
    pub(crate) fn confirm(&self, asap: Asap, status: ConStatus) {
        self.with(asap, |_, ram, _| {
            if ram.status() == TransmissionStatus::Transmitting {
                ram.set(GroupObjectRam::DATA_REQUEST, false);
                ram.set_status(match status {
                    ConStatus::Ok => TransmissionStatus::IdleOk,
                    ConStatus::NotOk => TransmissionStatus::IdleError,
                });
            }
        });
    }

    pub(crate) fn request_reads_on_init(&self) {
        let mut any = false;
        self.state.lock(|state| {
            let state = &mut *state.borrow_mut();
            for (ram, descriptor) in state.ram.iter_mut().zip(self.descriptors) {
                if descriptor.has(ConfigFlag::ReadOnInit) {
                    ram.set(GroupObjectRam::DATA_REQUEST, true);
                    ram.set_status(TransmissionStatus::TransmitRequest);
                    any = true;
                }
            }
        });
        if any {
            self.transmit_requested.signal(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ConfigFlag::*;
    use super::*;
    use crate::data_point::{B1, U16, U8};
    use crate::test_util::{frame, group_objects, init_pool, run};

    const SWITCH: Asap = Asap::new(1);
    const LEVEL: Asap = Asap::new(2);

    fn objects(switch: &[ConfigFlag], level: &[ConfigFlag]) -> &'static GroupObjects {
        group_objects(&[
            Descriptor::new(ValueType::bits(1), Priority::Low, switch),
            Descriptor::new(ValueType::octets(2), Priority::Normal, level),
        ])
    }

    fn write(apdu: &[u8]) -> Frame {
        frame(&[&[0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE0 | apdu.len() as u8, 0x00], apdu].concat())
    }

    #[test]
    fn descriptor_has_the_system_b_layout() {
        let all = [Communication, Read, Write, Transmit, Update, ReadOnInit];
        let d = Descriptor::new(ValueType::bits(1), Priority::Low, &all);
        assert_eq!(d.bits(), 0xFF00);
        let d = Descriptor::new(ValueType::octets(2), Priority::Normal, &[Communication, Write]);
        assert_eq!(d.bits(), 0x1508);
        assert_eq!(d.priority(), Priority::Normal);
        assert_eq!(d.value_type(), ValueType::octets(2));
        assert!(d.has(Write) && !d.has(Read));
    }

    #[test]
    fn value_type_sizes() {
        let cases = [
            (0, 1, 1),
            (6, 7, 1),
            (7, 8, 1),
            (14, 112, 14),
            (15, 40, 5),
            (20, 104, 13),
            (21, 120, 15),
            (254, 248 * 8, 248),
            (255, 252 * 8, 252),
        ];
        for (code, bits, octets) in cases {
            let t = ValueType::from_code(code);
            assert_eq!((t.size_bits(), t.size_octets()), (bits, octets), "code {code}");
            assert_eq!(t.is_short(), code <= 5, "code {code}");
        }
        assert_eq!(ValueType::octets(5).code(), 15);
        assert_eq!(ValueType::bits(7).code(), 6);
    }

    #[test]
    fn values_are_stored_back_to_back() {
        let d = [
            Descriptor::new(ValueType::bits(1), Priority::Low, &[]),
            Descriptor::new(ValueType::octets(2), Priority::Low, &[]),
            Descriptor::new(ValueType::octets(1), Priority::Low, &[]),
        ];
        assert_eq!(value_bytes(&d), 4);
        let gos = group_objects(Box::leak(Box::new(d)));
        gos.store(Asap::new(2), &U16::new(0x1234));
        gos.store(Asap::new(3), &U8::new(0xA5));
        gos.store(Asap::new(1), &B1::new(true));
        assert_eq!(gos.get::<U16>(Asap::new(2)), U16::new(0x1234));
        assert_eq!(gos.get::<U8>(Asap::new(3)), U8::new(0xA5));
    }

    #[test]
    #[should_panic]
    fn type_mismatch_panics() {
        objects(&[], &[]).get::<U8>(SWITCH);
    }

    #[test]
    fn set_sends_a_write_and_confirms() {
        init_pool();
        let gos = objects(&[Communication, Transmit], &[]);
        gos.set(SWITCH, &B1::new(true));
        assert_eq!(gos.status(SWITCH), TransmissionStatus::TransmitRequest);
        assert!(gos.transmit_pending());

        let (asap, f) = gos.start_transmission().unwrap();
        assert_eq!(asap, SWITCH);
        assert_eq!(f.group_service(), Some(GroupService::Write));
        assert_eq!(f.priority(), Priority::Low);
        assert_eq!(f.short_data(), 1);
        assert_eq!(gos.status(SWITCH), TransmissionStatus::Transmitting);
        assert!(gos.start_transmission().is_none());

        gos.confirm(SWITCH, ConStatus::Ok);
        assert_eq!(gos.status(SWITCH), TransmissionStatus::IdleOk);
        gos.set(SWITCH, &B1::new(false));
        gos.start_transmission().unwrap();
        gos.confirm(SWITCH, ConStatus::NotOk);
        assert_eq!(gos.status(SWITCH), TransmissionStatus::IdleError);
    }

    #[test]
    fn set_during_transmission_stays_pending() {
        init_pool();
        let gos = objects(&[Communication, Transmit], &[]);
        gos.set(SWITCH, &B1::new(true));
        gos.start_transmission().unwrap();
        gos.set(SWITCH, &B1::new(false));
        gos.confirm(SWITCH, ConStatus::Ok);
        assert_eq!(gos.status(SWITCH), TransmissionStatus::TransmitRequest);
        let (_, f) = gos.start_transmission().unwrap();
        assert_eq!(f.short_data(), 0);
    }

    #[test]
    fn transmit_request_needs_c_and_t() {
        init_pool();
        for flags in [&[Communication][..], &[Transmit]] {
            let gos = objects(flags, &[]);
            gos.set(SWITCH, &B1::new(true));
            assert!(gos.start_transmission().is_none());
            assert_eq!(gos.status(SWITCH), TransmissionStatus::IdleError);
        }
    }

    #[test]
    fn request_read_sends_a_read_once() {
        init_pool();
        let gos = objects(&[Communication, Transmit], &[]);
        gos.request_read(SWITCH);
        let (_, f) = gos.start_transmission().unwrap();
        assert_eq!(f.group_service(), Some(GroupService::Read));
        gos.confirm(SWITCH, ConStatus::Ok);

        gos.set(SWITCH, &B1::new(true));
        let (_, f) = gos.start_transmission().unwrap();
        assert_eq!(f.group_service(), Some(GroupService::Write));
    }

    #[test]
    fn reads_on_init_only_for_objects_with_i() {
        init_pool();
        let gos = objects(&[Communication, Transmit, ReadOnInit], &[Communication, Transmit]);
        gos.request_reads_on_init();
        let (asap, f) = gos.start_transmission().unwrap();
        assert_eq!((asap, f.group_service()), (SWITCH, Some(GroupService::Read)));
        assert!(gos.start_transmission().is_none());
    }

    #[test]
    fn bus_write_needs_c_and_w() {
        let w = write(&[0x81]);
        for (flags, stored) in [
            (&[Communication, Write][..], true),
            (&[Write], false),
            (&[Communication, Update], false),
        ] {
            let gos = objects(flags, &[]);
            assert_eq!(gos.bus_value(SWITCH, GroupService::Write, &w), stored, "{flags:?}");
            assert_eq!(gos.get::<B1>(SWITCH).value(), stored, "{flags:?}");
        }
    }

    #[test]
    fn bus_response_needs_c_and_u() {
        let response = frame(&[0xBC, 0x11, 0x0A, 0x09, 0x62, 0xE1, 0x00, 0x41]);
        let gos = objects(&[Communication, Write], &[]);
        assert!(!gos.bus_value(SWITCH, GroupService::Response, &response));
        let gos = objects(&[Communication, Update], &[]);
        assert!(gos.bus_value(SWITCH, GroupService::Response, &response));
        assert!(gos.get::<B1>(SWITCH).value());
    }

    #[test]
    fn bus_values_of_the_wrong_length_are_ignored() {
        let gos = objects(&[Communication, Write], &[Communication, Write]);
        assert!(!gos.bus_value(SWITCH, GroupService::Write, &write(&[0x80, 0x01])));
        assert!(!gos.bus_value(LEVEL, GroupService::Write, &write(&[0x80, 0x01])));
        assert!(!gos.bus_value(LEVEL, GroupService::Write, &write(&[0x81])));
        assert!(gos.bus_value(LEVEL, GroupService::Write, &write(&[0x80, 0x12, 0x34])));
        assert_eq!(gos.get::<U16>(LEVEL), U16::new(0x1234));
    }

    #[test]
    fn updated_yields_each_object_once() {
        let gos = objects(&[Communication, Write], &[Communication, Write]);
        gos.bus_value(LEVEL, GroupService::Write, &write(&[0x80, 0x12, 0x34]));
        gos.bus_value(SWITCH, GroupService::Write, &write(&[0x81]));
        run(async {
            assert_eq!(gos.updated().await, SWITCH);
            assert_eq!(gos.updated().await, LEVEL);
            let next = embassy_futures::poll_once(gos.updated());
            assert!(next.is_pending());
        });
    }

    #[test]
    fn read_response_needs_c_and_r() {
        init_pool();
        let gos = objects(&[Communication, Read], &[Read]);
        gos.store(SWITCH, &B1::new(true));
        let f = gos.response_frame(SWITCH).unwrap();
        assert_eq!(f.group_service(), Some(GroupService::Response));
        assert_eq!(f.short_data(), 1);
        assert!(gos.response_frame(LEVEL).is_none());
    }
}
