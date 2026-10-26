//! 视图：一段**已映射进本域**的内存。

/// 起点与长度成对才是一段区间——单独一个数拿出去没有意义，故两半私有、只能一起取走。
/// `Copy`：视图本来就是可复制的（两个线程各要一份正是既有用法）。
///
/// **它不承诺"用完即失效"**：交给谁就是把地址给出去了，而地址是裸数——可以被复制、存进
/// static、加偏移。撤图靠 [`crate::mapping::Dock::shut`]，那是礼节，不是证明。
#[derive(Clone, Copy, Debug)]
pub struct View {
    base: usize,
    size: usize,
}

impl View {
    pub(crate) fn new(base: usize, size: usize) -> Self {
        Self { base, size }
    }

    /// 视图起点。
    pub fn base(self) -> usize {
        self.base
    }

    /// 视图长度 ＝ **映射的那一段**（页对齐），不是设备声明的 `reg` 长度。
    pub fn size(self) -> usize {
        self.size
    }
}
