//! The only action/face/mark/path/mount table. No action shares an entrance.
use super::frame::Wire;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mount {
    Public,
    Bound,
    Installer,
}

macro_rules! entrances {
    ($($variant:ident = $code:literal, $name:literal, $mount:ident, ($pattern:pat);)+) => {
        crate::table! {
            pub enum Grant { $($variant => $name, ($pattern);)+ }
            stem: "identity-",
            name_max: 11,
            wire_ty: Wire,
        }
        impl Grant {
            pub const fn path(self) -> &'static str { match self { $(Self::$variant => concat!("/svc/sys/identity/", $name),)+ } }
            pub const fn mount(self) -> Mount { match self { $(Self::$variant => Mount::$mount,)+ } }
        }
        const _: () = { $(assert!(Grant::$variant.action() == $code);)+ };
    }
}
entrances! {
    Resolve = 1, "resolve", Public, (Wire::Resolve(_));
    Matches = 2, "matches", Public, (Wire::Matches(..));
    Same = 3, "same", Public, (Wire::Same(..));
    Sire = 4, "sire", Public, (Wire::Sire(_));
    Heir = 5, "heir", Public, (Wire::Heir(..));
    Amid = 6, "amid", Public, (Wire::Amid(..));
    Members = 7, "members", Public, (Wire::Members(..));
    Memberships = 8, "memberships", Public, (Wire::Memberships(..));
    Adopt = 9, "adopt", Bound, (Wire::Adopt(_));
    Waive = 10, "waive", Bound, (Wire::Waive);
    Restrict = 11, "restrict", Bound, (Wire::Restrict(_));
    Derive = 12, "derive", Bound, (Wire::Derive(_));
    Found = 13, "found", Bound, (Wire::Found);
    Admit = 14, "admit", Bound, (Wire::Admit(..));
    Expel = 15, "expel", Bound, (Wire::Expel(..));
    Bind = 16, "bind", Installer, (Wire::Bind(..));
    Unbind = 17, "unbind", Installer, (Wire::Unbind(_));
}
