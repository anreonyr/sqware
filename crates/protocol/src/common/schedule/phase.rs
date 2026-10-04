#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    PreMint,
    Mint,
    PostMint,
    PreEmbark,
    Embark,
    PostEmbark,
    PreDebark,
    Debark,
    PostDebark,
    PreRuin,
    Ruin,
    PostRuin,
}
