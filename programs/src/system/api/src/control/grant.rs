//! Stable Control capability table.

use env::{Mark, marks::Definition};
use super::frame::Wire;

macro_rules! entrances {
    ($($variant:ident = $at:literal => $name:literal, ($pattern:pat);)+) => {
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Grant { $($variant = $at - 1,)+ }

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
                match self { $(Self::$variant => Mark::of(concat!("control-entry-", $name)),)+ }
            }
            pub const MARKS: [Mark; Self::COUNT] = [$(Self::$variant.mark(),)+];
            pub const DECLARATIONS: [Definition; Self::COUNT] = [
                $(Definition { name: concat!("control-entry-", $name), mark: Self::$variant.mark() },)+
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
    State = 1 => "state", (Wire::State(_) | Wire::StateInstance(_));
    Mint = 2 => "mint", (Wire::Mint(_));
    Embark = 3 => "embark", (Wire::Embark(_) | Wire::EmbarkInstance(_));
    Debark = 4 => "debark", (Wire::Debark(_) | Wire::DebarkInstance(_));
    Ruin = 5 => "ruin", (Wire::Ruin(_) | Wire::RuinInstance(_));
}
