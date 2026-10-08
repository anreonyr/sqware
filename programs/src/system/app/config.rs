use crate::boot::Catalog;
pub(crate) fn account(catalog: Catalog<'static>) -> crate::system::account::Configuration {
    crate::system::account::Configuration {
        name: "anran",
        image: catalog.find("cat").map(|entry| entry.elf),
    }
}
