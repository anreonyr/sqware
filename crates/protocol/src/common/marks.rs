//! Mark declarations and collision checks, independent of protocol domains.
pub use ::wire::marks::{Definition, conflict};

#[macro_export]
macro_rules! marks {
    ($($(#[$meta:meta])* $vis:vis const $name:ident $(, $text:ident)? = $value:literal;)*) => {
        $($(#[$meta])* $vis const $name: env::Mark = env::Mark::of($value);
            $($vis const $text: &str = $value;)?
        )*
        pub(crate) const DECLARATIONS: &[$crate::common::marks::Definition] = &[
            $($crate::common::marks::Definition { name: $value, mark: $name },)*
        ];
        const _: () = {
            $(assert!($name.get() != env::Mark::NONE.get(), "reserved empty mark");)*
            assert!($crate::common::marks::conflict(&[DECLARATIONS]).is_none(), "duplicate mark");
        };
    };
}
