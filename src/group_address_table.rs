use crate::frame::GroupAddress;

/// Index into the group address table, starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Tsap(u16);

impl Tsap {
    pub const fn new(number: u16) -> Self {
        assert!(number > 0);
        Self(number)
    }
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// The device's group addresses, in strictly ascending order.
#[derive(Debug, Clone, Copy)]
pub struct GroupAddressTable(&'static [GroupAddress]);

impl GroupAddressTable {
    pub const fn new(addresses: &'static [GroupAddress]) -> Self {
        let mut i = 1;
        while i < addresses.len() {
            assert!(
                addresses[i - 1].get() < addresses[i].get(),
                "group addresses must be sorted"
            );
            i += 1;
        }
        Self(addresses)
    }

    pub fn tsap(&self, address: &GroupAddress) -> Option<Tsap> {
        let index = self.0.binary_search(address).ok()?;
        Some(Tsap(index as u16 + 1))
    }

    pub fn address(&self, tsap: Tsap) -> Option<GroupAddress> {
        self.0.get(tsap.0 as usize - 1).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ADDRESSES: [GroupAddress; 3] = [
        GroupAddress::from_parts(1, 1, 98),
        GroupAddress::from_parts(1, 1, 99),
        GroupAddress::from_parts(2, 0, 1),
    ];

    #[test]
    fn maps_addresses_to_tsaps_and_back() {
        let table = GroupAddressTable::new(&ADDRESSES);
        for (i, address) in ADDRESSES.iter().enumerate() {
            let tsap = table.tsap(address).unwrap();
            assert_eq!(tsap, Tsap::new(i as u16 + 1));
            assert_eq!(table.address(tsap), Some(*address));
        }
        assert_eq!(table.tsap(&GroupAddress::from_parts(1, 1, 100)), None);
        assert_eq!(table.address(Tsap::new(4)), None);
    }

    #[test]
    #[should_panic(expected = "sorted")]
    fn rejects_unsorted_addresses() {
        static UNSORTED: [GroupAddress; 2] =
            [GroupAddress::from_parts(1, 1, 99), GroupAddress::from_parts(1, 1, 98)];
        GroupAddressTable::new(&UNSORTED);
    }
}
