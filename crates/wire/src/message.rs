/// A bounded encoded message and its decoded form.
pub trait Message {
    type In;
    type Buf: AsRef<[u8]> + AsMut<[u8]>;
    const EMPTY: Self::Buf;
    const MAX: usize = core::mem::size_of::<Self::Buf>();

    fn store(&self, out: &mut [u8]) -> Option<usize>;
    fn fetch(bytes: &[u8]) -> Option<Self::In>;
}
