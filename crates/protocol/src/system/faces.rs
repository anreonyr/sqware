//! `faces!` —— **一族的面**：枚举 ＋ `ALL` ＋ 位次 ＋ 记号 ＋ 认面 ＋ 那组编译期断言，一次生成。
//!
//! ```text
//!   faces! { … }  ⇒  enum Grant { … }                              ← 变体（各自一段名字）
//!                    impl Grant { COUNT, ALL, at, of_wire, name, mark, MARKS }
//!                    pub fn grant_of(mark) -> Option<Grant>
//!                    const _: () = { … };                          ← 位次对齐 / 本族记号两两不相撞
//! ```
//!
//! # 这一台是**数出来**才抽的（照实记）
//!
//! 仓里那条规矩是"**两台以上逐字同构 ⇒ 收**"（`board/client.rs` 那句原文），故第一台
//! （[`operator::grant`](crate::system::operator::grant)，七位）落地时**不抽**——一台就抽是投机。
//! 第二台（[`principal::grant`](crate::service::principal::grant)，两面）落地之后**逐行量了一次**：
//! 两份去掉注释是 **100 行与 80 行**，逐行比**只有 50 行不同，而那 50 行全是各家自己的事实**
//! （哪几个变体、哪条线上码落哪一面、记号词根）；`at()` / `mark()` 那套 const-fn
//! 拼缓冲 / `grant_of` / 断言骨架**逐字同构**。⇒ 抽的判据成立，抽走的正是那"逐字同构"的一半。
//!
//! # 一族要交代的只有**它自己的事实**
//!
//! | 参数 | 是什么 |
//! |---|---|
//! | 变体列（每个带一段名字） | 这一族有哪几条权柄边界 |
//! | `stem` | 记号词根——面名拼在它后面（**面名只有一处**） |
//! | `name_max` | 面名最长几字节（定长缓冲要多大；**逐变体断言**，写小了当场编不过） |
//! | `wire_ty` ＋ `wire { … }` | 哪条线上码落哪一面（**穷尽 `match`**：加一条线上动作不补这里 ⇒ 编不过） |
//!
//! 剩下的（位次怎么算、记号怎么拼、认面怎么扫、断言怎么排）**只此一份**。
//!
//! # 跨族那一条**不在这里**（这一刀收了它）
//!
//! 从前还有第六个参数 `distinct: [...]`：这一族的记号还要与**哪几枚**不撞——而"哪几枚"写的是
//! 别族的记号**字面量**。那是一张 **O(族数²)** 的手抄表，而且**覆盖不全**：实测三处漏——
//! `principal-back` 与 `coalition-back` 从未两两判过，`"entry"` 与 `"tip"` 之间没有，
//! `operator-ask` 与别族的面也没有。今天各族只吐**自己**那几枚（[`Grant::MARKS`]），
//! 全协议那一判只有一处：[`crate::system`] 的全族总表。
//!
//! **照实记（`wire` 那一格一条线上码一行，不并 `|`）**：`macro_rules` 的 `:pat` 不吃顶层的
//! `|`（那一条留给了 `$a:pat | $b:pat`），并起来就得加括号——而展开之后那对括号又是多余的
//! （`unused_parens` 当场报警）。**故不并**：一条线上码一行，各说自己落哪一面。多条共用一个
//! 面时读起来更啰嗦，换的是**机制里没有那条口径**，且那一张对照表逐条 grep 得到。
//!
//! **照实记（这一份注差点自己犯"同一件事说两遍"）**：初版在这里还列了一张"生成的那几格各自
//! 一句"——而 `COUNT` / `ALL` / `at` / `of_wire` / `name` / `mark` / `grant_of` **自己就带着那几
//! 段注**（下面那段 `macro_rules` 里逐条写着）。那张表整段删：一份注说一遍。
//!
//! # 展开要用到的名字由本文件自己带（照实记）
//!
//! 宏体里每一处都写 `$crate::system::faces::…` **全路径**（[`Mark`] 由本文件转出）。从前它写裸
//! `Mark`，于是**每一个调用点都得先 `use env::Mark;` 而自己一处都不用**——那是一条藏在宏里的
//! 要求：删掉那行 import 就在调用点报"找不到 `Mark`"。今天调用点一个名字都不用带。

/// 宏展开要用到的记号类型——**由本文件转出**，故调用点不必 import（见上面那一份照实记）。
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
            /// 几位——**由变体列表达出来，不另写数**（另写一个数就要靠自律对齐）。
            ///
            /// **它不进公共面**（私有）：读者只有本文件里的 [`Grant::at`] 与下面那张
            /// [`Grant::ALL`] 的长度——外面那一侧要"几位"就直接 `ALL.len()`。
            const COUNT: usize = [$( $crate::faces!(@unit $Variant) ),*].len();

            /// 次序即**位次**（第 i 位在 `ALL[i-1]`）。
            ///
            /// 位次由这一张表给，故 [`Grant::at`] 不另写一张：**改枚举次序那条路根本不存在**。
            pub const ALL: [$Grant; Self::COUNT] = [$( $Grant::$Variant,)*];

            /// **这一位是几**（1..=COUNT）——判面那一句比的就是它。
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

            /// **这一问落哪一面**——穷尽 `match`：加一条线上动作不补这里 ⇒ **编不过**。
            pub const fn of_wire(wire: &$Wire) -> u8 {
                match wire {
                    $($wpat => $Grant::$wvar.at(),)*
                }
            }

            /// 这一面叫什么（**树上那一段名字**：`/svc/<族>/{name}`）。
            pub const fn name(self) -> &'static str {
                match self {
                    $($Grant::$Variant => $name,)*
                }
            }

            /// 这一面的记号（**入口 Pie 与门牌两侧同一个**）。
            ///
            /// **两段在这里拼一次**：词根 ＋ [`Grant::name`] 那一段——面名只有一处，记号不抄第二遍
            /// 字面量。`const fn` 里拼不出 `&str`、也切不出 `&[u8]`，故那两段落进一块定长缓冲、按
            /// **实际长度**交给 [`Mark::of_bytes`]（同一条 FNV-1a，两侧各算同一个数）。
            ///
            /// 缓冲够不够由下面那句 `assert!` 钉住：面名比 `name_max` 长 ⇒ **当场编不过**。
            pub const fn mark(self) -> $crate::system::faces::Mark {
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
                $crate::system::faces::Mark::of_bytes(&buf, n)
            }

            /// **本族所有面的记号**（一行一族）——"**全协议记号两两不相撞**"那一张总表读它。
            ///
            /// **它为什么在这里**（照实记：这一格是从各族那份手抄清单收来的）：从前那一条由各族
            /// 自己写 `distinct: [...]`——把"别族所有的记号"抄一遍。那是一张 **O(族数²)** 的手抄
            /// 表，且**覆盖不全**：实测 `principal-back` 与 `coalition-back` 从未两两判过，
            /// `"entry"` 与 `"tip"` 之间也没有，`operator-ask` 与别族的面也没有。今天各族只报**自己**
            /// 那几枚，全表那一次判在 [`crate::system`]（它说得全"全协议有哪些记号"）。
            pub const MARKS: [$crate::system::faces::Mark; Self::COUNT] = [$( $Grant::$Variant.mark(), )*];
        }

        /// **认面**：这枚记号是哪一面。
        ///
        /// **答 `None` 不是"失败"**，是"这一枚不是本族的面"——故服务端据此**不判面**
        /// （各族"还有哪一种孔走到这儿"的正文归各族自己的文件头）。
        pub fn grant_of(mark: $crate::system::faces::Mark) -> Option<$Grant> {
            let mut i = 0;
            while i < $Grant::ALL.len() {
                if $Grant::ALL[i].mark().get() == mark.get() {
                    return Some($Grant::ALL[i]);
                }
                i += 1;
            }
            None
        }

        // ── 位次对齐 ＋ 本族记号两两不相撞（**编译期**钉住）────────────────
        //
        // 那几枚记号各是一枚散列，**撞了就是那次装机塌掉**（`ASK_MARK` 的照实记里那次实测
        // 0/3 就是这么来的）。比的是 `.get()` 那个裸值：`Mark` 的 `PartialEq` 不是 `const`，
        // 而 `get` 是 `const fn`。
        //
        // **跨族那一半不在这里**（照实记）：它从前靠各族手抄一份"别族的记号"，而那张表
        // **覆盖不全**（实测三处漏，见 [`Grant::MARKS`]）。今天全协议那一判只有一处——
        // [`crate::system`] 的全族总表，它读的是下面这一枚 `MARKS`。
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
