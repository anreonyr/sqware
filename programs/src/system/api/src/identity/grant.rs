//! Identity actions, mounts, and their stable marks.

use env::{Mark, marks::Definition};
use super::frame::Wire;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mount { Public, Bound, Installer }

macro_rules! entrances {
    ($($variant:ident = $action:literal => $name:literal, $mount:ident, ($pattern:pat);)+) => {
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Grant { $($variant = $action - 1,)+ }

        impl Grant {
            pub const COUNT: usize = [$(stringify!($variant)),+].len();
            pub const ALL: [Self; Self::COUNT] = [$(Self::$variant,)+];

            pub const fn at(self) -> u8 { self as u8 + 1 }
            pub const fn index(self) -> usize { self.at() as usize - 1 }
            pub const fn from_action(action: u8) -> Option<Self> {
                if action == 0 || action as usize > Self::COUNT { None }
                else { Some(Self::ALL[action as usize - 1]) }
            }
            pub const fn for_wire(wire: &Wire) -> Self {
                match wire { $($pattern => Self::$variant,)+ }
            }
            pub const fn of_wire(wire: &Wire) -> u8 { Self::for_wire(wire).at() }
            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name,)+ }
            }
            pub const fn mark(self) -> Mark {
                match self { $(Self::$variant => Mark::of(concat!("identity-", $name)),)+ }
            }
            pub const fn mount(self) -> Mount {
                match self { $(Self::$variant => Mount::$mount,)+ }
            }
            pub const MARKS: [Mark; Self::COUNT] = [$(Self::$variant.mark(),)+];
            pub const DECLARATIONS: [Definition; Self::COUNT] = [
                $(Definition { name: concat!("identity-", $name), mark: Self::$variant.mark() },)+
            ];
        }

        pub fn grant_of(mark: Mark) -> Option<Grant> {
            Grant::ALL.iter().copied().find(|grant| grant.mark() == mark)
        }

        const _: () = {
            let all = Grant::ALL;
            assert!(all.len() <= u8::MAX as usize);
            let mut i = 0;
            while i < all.len() {
                assert!(all[i].at() == (i + 1) as u8);
                let mut j = i + 1;
                while j < all.len() {
                    assert!(all[i].mark().get() != all[j].mark().get());
                    j += 1;
                }
                i += 1;
            }
        };
    };
}

entrances! {
    Resolve = 1 => "resolve", Public, (Wire::Resolve(_));
    Matches = 2 => "matches", Public, (Wire::Matches(..));
    Same = 3 => "same", Public, (Wire::Same(..));
    Sire = 4 => "sire", Public, (Wire::Sire(_));
    Heir = 5 => "heir", Public, (Wire::Heir(..));
    Amid = 6 => "amid", Public, (Wire::Amid(..));
    Members = 7 => "members", Public, (Wire::Members(..));
    Memberships = 8 => "memberships", Public, (Wire::Memberships(..));
    Adopt = 9 => "adopt", Bound, (Wire::Adopt(_));
    Waive = 10 => "waive", Bound, (Wire::Waive);
    Restrict = 11 => "restrict", Bound, (Wire::Restrict(_));
    Derive = 12 => "derive", Bound, (Wire::Derive(_));
    Found = 13 => "found", Bound, (Wire::Found);
    Admit = 14 => "admit", Bound, (Wire::Admit(..));
    Expel = 15 => "expel", Bound, (Wire::Expel(..));
    Bind = 16 => "bind", Installer, (Wire::Bind(..));
    Unbind = 17 => "unbind", Installer, (Wire::Unbind(_));
}

const _: () = assert!(env::marks::conflict(&[&Grant::DECLARATIONS]).is_none());
