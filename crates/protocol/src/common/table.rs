//! 一张表说尽一族面：变体（位次）· 名字 · 记号 · 线上那一形。
//! **一形**：线网形就写在变体那一行里（`变体 => "名", (线网形);`）——不另立一块列：
//! 另立一块就要靠自律对齐，且漏一枚变体不报错。

pub use env::Mark;

#[macro_export]
macro_rules! table {
    (
        $(#[$meta:meta])*
        $vis:vis enum $Grant:ident {
            $($(#[$vmeta:meta])* $Variant:ident => $name:literal, ($wpat:pat);)*
        }
        stem: $stem:literal,
        wire_ty: $Wire:ty,
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        $vis enum $Grant {
            $($(#[$vmeta])* $Variant,)*
        }

        impl $Grant {
/// 几位——**由变体列表达出来，不另写数**（另写一个数就要靠自律对齐）
            pub const COUNT: usize = [$( stringify!($Variant) ),*].len();

/// 次序即**位次**（第 i 位在 `ALL[i-1]`）
/// 位次由这一张表给，故 Grant::at 不另写一张：**改枚举次序那条路根本不存在**
            pub const ALL: [$Grant; Self::COUNT] = [$( $Grant::$Variant,)*];

/// **这一位是几**（1..=COUNT）——判别式就是位次，下面那句断言把两者钉在一起
            pub const fn at(self) -> u8 { self as u8 + 1 }

            pub const fn of_wire(wire: &$Wire) -> u8 {
                Self::for_wire(wire).at()
            }

            pub const fn for_wire(wire: &$Wire) -> Self {
                match wire {
                    $($wpat => $Grant::$Variant,)*
                }
            }

            pub const fn index(self) -> usize { self.at() as usize - 1 }
            pub const fn from_action(action: u8) -> Option<Self> {
                if action == 0 || action as usize > Self::COUNT {
                    None
                } else {
                    Some(Self::ALL[action as usize - 1])
                }
            }

/// 这一面叫什么（**树上那一段名字**：`/svc/<族>/{name}`）
            pub const fn name(self) -> &'static str {
                match self {
                    $($Grant::$Variant => $name,)*
                }
            }

/// 这一面的记号（**入口 Pie 与门牌两侧同一个**）
/// 字面量。`const fn` 里拼不出 `&str`、也切不出 `&[u8]`，故那两段落进一块定长缓冲、按
/// **实际长度**交给 Mark::of_bytes（同一条 FNV-1a，两侧各算同一个数）
/// 缓冲够不够不另写数：`NAME_MAX` 就是名字列里最长的那一段（写死一个数就要靠自律对齐）
            pub const fn mark(self) -> $crate::common::table::Mark {
                const STEM: &[u8] = $stem.as_bytes();
                const NAME_MAX: usize = {
                    let mut m = 0;
                    $({ let l = $name.len(); if l > m { m = l; } })*
                    m
                };
                let rest = self.name().as_bytes();
                let mut buf = [0u8; STEM.len() + NAME_MAX];
                let mut n = 0;
                while n < STEM.len() {
                    buf[n] = STEM[n];
                    n += 1;
                }
                let mut j = 0;
                while j < rest.len() {
                    buf[n] = rest[j];
                    n += 1;
                    j += 1;
                }
                $crate::common::table::Mark::of_bytes(&buf, n)
            }

/// **本族所有面的记号**（一行一族）——"**全协议记号两两不相撞**"那一张总表读它
            pub const MARKS: [$crate::common::table::Mark; Self::COUNT] = [$( $Grant::$Variant.mark(), )*];
        }

/// **认面**：这枚记号是哪一面
/// **答 `None` 不是"失败"**，是"这一枚不是本族的面"——故服务端据此**不判面**
/// （各族"还有哪一种孔走到这儿"的正文归各族自己的文件头）
        pub fn grant_of(mark: $crate::common::table::Mark) -> Option<$Grant> {
            let mut i = 0;
            while i < $Grant::ALL.len() {
                if $Grant::ALL[i].mark().get() == mark.get() {
                    return Some($Grant::ALL[i]);
                }
                i += 1;
            }
            None
        }

        const _: () = {
            let all = $Grant::ALL;
            assert!(all.len() <= u8::MAX as usize);
            let mut i = 0;
            while i < all.len() {
                // `at()` 读的是判别式，`ALL` 是同一个重复生成的：两者**必须**逐位对齐
                // （谁写了显式判别式、或让 `ALL` 错序 ⇒ 当场编不过）。
                assert!(all[i].at() == (i + 1) as u8);
                // 本族记号两两不相撞。
                let mut j = i + 1;
                while j < all.len() {
                    assert!(all[i].mark().get() != all[j].mark().get());
                    j += 1;
                }
                i += 1;
            }
        };
    };
}
