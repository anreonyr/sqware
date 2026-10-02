//! The only action/face/mark/mount table. No action shares an entrance.
//! **位次即动作码**（第 i 行 ⇒ 第 i 号：`table!` 的 `at()` 就是它）——故这里不写第二个数。
use super::frame::Wire;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mount {
    Public,
    Bound,
    Installer,
}

macro_rules! entrances {
    ($($variant:ident, $name:literal, $mount:ident, ($pattern:pat);)+) => {
        crate::table! {
            pub enum Grant { $($variant => $name, ($pattern);)+ }
            stem: "identity-",
            name_max: 11,
            wire_ty: Wire,
        }
        impl Grant {
            pub const fn mount(self) -> Mount { match self { $(Self::$variant => Mount::$mount,)+ } }
        }
    }
}
entrances! {
    Resolve, "resolve", Public, (Wire::Resolve(_));
    Matches, "matches", Public, (Wire::Matches(..));
    Same, "same", Public, (Wire::Same(..));
    Sire, "sire", Public, (Wire::Sire(_));
    Heir, "heir", Public, (Wire::Heir(..));
    Amid, "amid", Public, (Wire::Amid(..));
    Members, "members", Public, (Wire::Members(..));
    Memberships, "memberships", Public, (Wire::Memberships(..));
    Adopt, "adopt", Bound, (Wire::Adopt(_));
    Waive, "waive", Bound, (Wire::Waive);
    Restrict, "restrict", Bound, (Wire::Restrict(_));
    Derive, "derive", Bound, (Wire::Derive(_));
    Found, "found", Bound, (Wire::Found);
    Admit, "admit", Bound, (Wire::Admit(..));
    Expel, "expel", Bound, (Wire::Expel(..));
    Bind, "bind", Installer, (Wire::Bind(..));
    Unbind, "unbind", Installer, (Wire::Unbind(_));
}
