//! Compatibility syntax for entrance tables.
pub use env::Mark;

#[macro_export]
macro_rules! faces {
    ($($table:tt)*) => { $crate::table! { $($table)* } };
}
