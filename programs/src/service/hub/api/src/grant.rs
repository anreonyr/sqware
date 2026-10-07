use env::{Mark, marks::Definition};
use crate::frame::Wire;

macro_rules! entrances {
    ($($variant:ident => $name:literal, ($pattern:pat);)+) => {
        #[repr(u8)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Grant { $($variant,)+ }

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
                Mark::of(match self { $(Self::$variant => concat!("hub-entry-", $name),)+ })
            }
            pub const MARKS: [Mark; Self::COUNT] = [$(Self::$variant.mark(),)+];
            pub const DECLARATIONS: [Definition; Self::COUNT] = [
                $(Definition { name: concat!("hub-entry-", $name), mark: Self::$variant.mark() },)+
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
                assert!(all[i].mark().get() != Mark::NONE.get());
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
    Bond => "bond", (Wire::Bond(_));
    List => "list", (Wire::List(_, _));
    Claim => "claim", (Wire::Claim { .. });
}

const _: () = {
    assert!(crate::frame::BACK_MARK.get() != Grant::Bond.mark().get());
    assert!(crate::frame::BACK_MARK.get() != Grant::List.mark().get());
    assert!(crate::frame::BACK_MARK.get() != Grant::Claim.mark().get());
    assert!(crate::frame::ALIVE_MARK.get() != Grant::Bond.mark().get());
    assert!(crate::frame::ALIVE_MARK.get() != Grant::List.mark().get());
    assert!(crate::frame::ALIVE_MARK.get() != Grant::Claim.mark().get());
    assert!(crate::frame::ALIVE_MARK.get() != crate::frame::BACK_MARK.get());
    assert!(crate::frame::ALIVE_MARK.get() != Mark::NONE.get());
};
