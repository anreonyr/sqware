//! A bounded single path component used in typed publication and account requests.
pub fn valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 31
        && !name.contains('/')
        && !name.as_bytes().contains(&0)
        && name != "."
        && name != ".."
        && !name.chars().any(char::is_control)
}
