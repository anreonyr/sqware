// 位标志宏 — 生成类型安全的 bitflag 集合类型（自包含副本）
//
// 原依赖内核 macros/mod.rs 的 `bitflags!`（#[macro_export] 到 crate root），
// 现复制进本模块经 `#[macro_use]` 引入——复制 memory 到其他项目无需内核 macros。
//
// 生成的类型自动获得 |、&、!、|=、&=、^= 运算符，
// 以及 contains()、intersects()、insert()、remove()、toggle() 方法。

macro_rules! bitflags {
    (
        $(#[$outer:meta])*
        $vis:vis struct $BitFlags:ident: $T:ty {
            $(
                $(#[$inner:meta])*
                const $Flag:ident = $value:expr;
            )*
        }
    ) => {
        $(#[$outer])*
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis struct $BitFlags($T);

        // bitflags 宏生成完整标志集 API；内核当前只用 contains/bits/empty 等子集，
        // 其余方法（from_bits/intersects/insert/remove/toggle/set）为公共 API 保留。
        #[allow(dead_code)]
        impl $BitFlags {
            $(
                $(#[$inner])*
                pub const $Flag: Self = Self($value);
            )*

            /// 空标志集（所有位清零）
            #[inline(always)]
            pub const fn empty() -> Self {
                Self(0)
            }

            /// 从原始整数值构造（不验证非法位）
            #[inline(always)]
            pub const fn from_bits(bits: $T) -> Self {
                Self(bits)
            }

            /// 返回原始整数值
            #[inline(always)]
            pub const fn bits(self) -> $T {
                self.0
            }

            /// 所有指定的标志都已置位
            #[inline(always)]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            /// 任一指定的标志已置位
            #[inline(always)]
            pub const fn intersects(self, other: Self) -> bool {
                self.0 & other.0 != 0
            }

            /// 设置指定标志
            #[inline(always)]
            pub fn insert(&mut self, other: Self) {
                self.0 |= other.0;
            }

            /// 清除指定标志
            #[inline(always)]
            pub fn remove(&mut self, other: Self) {
                self.0 &= !other.0;
            }

            /// 翻转指定标志
            #[inline(always)]
            pub fn toggle(&mut self, other: Self) {
                self.0 ^= other.0;
            }

            /// 替换为新的标志集
            #[inline(always)]
            pub fn set(&mut self, other: Self) {
                self.0 = other.0;
            }
        }

        // ── 运算符 ──────────────────────────────────

        impl core::ops::BitOr for $BitFlags {
            type Output = Self;
            #[inline(always)]
            fn bitor(self, rhs: Self) -> Self {
                Self(self.0 | rhs.0)
            }
        }

        impl core::ops::BitOrAssign for $BitFlags {
            #[inline(always)]
            fn bitor_assign(&mut self, rhs: Self) {
                self.0 |= rhs.0;
            }
        }

        impl core::ops::BitAnd for $BitFlags {
            type Output = Self;
            #[inline(always)]
            fn bitand(self, rhs: Self) -> Self {
                Self(self.0 & rhs.0)
            }
        }

        impl core::ops::BitAndAssign for $BitFlags {
            #[inline(always)]
            fn bitand_assign(&mut self, rhs: Self) {
                self.0 &= rhs.0;
            }
        }

        impl core::ops::BitXor for $BitFlags {
            type Output = Self;
            #[inline(always)]
            fn bitxor(self, rhs: Self) -> Self {
                Self(self.0 ^ rhs.0)
            }
        }

        impl core::ops::BitXorAssign for $BitFlags {
            #[inline(always)]
            fn bitxor_assign(&mut self, rhs: Self) {
                self.0 ^= rhs.0;
            }
        }

        impl core::ops::Not for $BitFlags {
            type Output = Self;
            #[inline(always)]
            fn not(self) -> Self {
                Self(!self.0)
            }
        }

        // ── 格式化 ──────────────────────────────────

        impl core::fmt::Debug for $BitFlags {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}(", stringify!($BitFlags))?;
                let mut first = true;
                $(
                    if self.contains(Self::$Flag) {
                        if !first {
                            write!(f, "|")?;
                        }
                        write!(f, "{}", stringify!($Flag))?;
                        first = false;
                    }
                )*
                if first {
                    write!(f, "0x0")?;
                }
                write!(f, ")")
            }
        }

        impl core::fmt::LowerHex for $BitFlags {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::LowerHex::fmt(&self.0, f)
            }
        }

        impl core::fmt::UpperHex for $BitFlags {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::UpperHex::fmt(&self.0, f)
            }
        }

        impl core::default::Default for $BitFlags {
            fn default() -> Self {
                Self::empty()
            }
        }
    };
}
