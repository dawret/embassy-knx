use crate::group_address_table::Tsap;
use crate::group_object::Asap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Association {
    pub tsap: Tsap,
    pub asap: Asap,
}

impl Association {
    pub const fn new(tsap: u16, asap: u16) -> Self {
        Self {
            tsap: Tsap::new(tsap),
            asap: Asap::new(asap),
        }
    }
}

/// Links group addresses (TSAPs) to group objects (ASAPs), n:m.
#[derive(Debug, Clone, Copy)]
pub struct AssociationTable(&'static [Association]);

impl AssociationTable {
    pub const fn new(associations: &'static [Association]) -> Self {
        Self(associations)
    }

    /// Every group object a telegram on `tsap` is for.
    pub fn asaps(&self, tsap: Tsap) -> impl Iterator<Item = Asap> + 'static {
        self.0
            .iter()
            .filter(move |a| a.tsap == tsap)
            .map(|a| a.asap)
    }

    /// The group address an object sends on: its first association.
    pub fn sending_tsap(&self, asap: Asap) -> Option<Tsap> {
        self.0.iter().find(|a| a.asap == asap).map(|a| a.tsap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ASSOCIATIONS: [Association; 4] = [
        Association::new(1, 1),
        Association::new(2, 2),
        Association::new(1, 3),
        Association::new(3, 2),
    ];

    #[test]
    fn one_tsap_reaches_every_associated_object() {
        let table = AssociationTable::new(&ASSOCIATIONS);
        let asaps: Vec<_> = table.asaps(Tsap::new(1)).collect();
        assert_eq!(asaps, [Asap::new(1), Asap::new(3)]);
        assert_eq!(table.asaps(Tsap::new(4)).count(), 0);
    }

    #[test]
    fn objects_send_on_their_first_association() {
        let table = AssociationTable::new(&ASSOCIATIONS);
        assert_eq!(table.sending_tsap(Asap::new(2)), Some(Tsap::new(2)));
        assert_eq!(table.sending_tsap(Asap::new(3)), Some(Tsap::new(1)));
        assert_eq!(table.sending_tsap(Asap::new(4)), None);
    }
}
