pub struct Install {
    pub delivery: super::Delivery,
    pub(crate) bootstrap: Option<super::bootstrap::Carrier>,
}
