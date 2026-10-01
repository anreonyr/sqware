//! faces!

pub use env::Mark;

#[macro_export]
macro_rules! faces {
    // 内部那一格：把一个变体折成一个 `()`——只为数得出一行有几个（`COUNT` 用）。
    (@unit $x:ident) => {
        ()
    };
    (
        $(#[$meta:meta])*
        $vis:vis enum $Grant:ident {
            $($(#[$vmeta:meta])* $Variant:ident => $name:literal,)*
        }
        stem: $stem:literal,
        name_max: $name_max:literal,
        wire_ty: $Wire:ty,
        wire: {
            $($wpat:pat => $wvar:ident,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        $vis enum $Grant {
            $($(#[$vmeta])* $Variant,)*
        }

        impl $Grant {
/// 几位——**由变体列表达出来，不另写数**（另写一个数就要靠自律对齐）
            const COUNT: usize = [$( $crate::faces!(@unit $Variant) ),*].len();

/// 次序即**位次**（第 i 位在 `ALL[i-1]`）
/// 位次由这一张表给，故 Grant::at 不另写一张：**改枚举次序那条路根本不存在**
            pub const ALL: [$Grant; Self::COUNT] = [$( $Grant::$Variant,)*];

/// **这一位是几**（1..=COUNT）——判面那一句比的就是它
            pub const fn at(self) -> u8 {
                let mut i = 0;
                while i < Self::COUNT {
                    if Self::ALL[i] as u8 == self as u8 {
                        return (i + 1) as u8;
                    }
                    i += 1;
                }
                // 不可能：每一位都在 `ALL` 里（加变体不补 `ALL` ⇒ 这条循环走到底）。
                panic!(concat!(stringify!($Grant), ": not in ALL"))
            }

            pub const fn of_wire(wire: &$Wire) -> u8 {
                match wire {
                    $($wpat => $Grant::$wvar.at(),)*
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
/// 缓冲够不够由下面那句 `assert!` 钉住：面名比 `name_max` 长 ⇒ **当场编不过**
            pub const fn mark(self) -> $crate::common::faces::Mark {
                const STEM: &[u8] = $stem.as_bytes();
                let rest = self.name().as_bytes();
                assert!(rest.len() <= $name_max, concat!(stringify!($Grant), ": face name too long"));
                let mut buf = [0u8; STEM.len() + $name_max];
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
                $crate::common::faces::Mark::of_bytes(&buf, n)
            }

/// **本族所有面的记号**（一行一族）——"**全协议记号两两不相撞**"那一张总表读它
            pub const MARKS: [$crate::common::faces::Mark; Self::COUNT] = [$( $Grant::$Variant.mark(), )*];
        }

/// **认面**：这枚记号是哪一面
/// **答 `None` 不是"失败"**，是"这一枚不是本族的面"——故服务端据此**不判面**
/// （各族"还有哪一种孔走到这儿"的正文归各族自己的文件头）
        pub fn grant_of(mark: $crate::common::faces::Mark) -> Option<$Grant> {
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
            let mut i = 0;
            while i < all.len() {
                // 位次必须逐位对齐（`ALL` 就是位次表）。
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
