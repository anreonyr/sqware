//! 不归某族"面"那一族、却被当记号用的那几枚，与三族的面
//! 逐对判一次"全协议任两枚不许撞"。这一处看得见整棵树，故由它钉。

use crate::system::control;

/// **面之外那几枚记号**（不归某族"面"那一族、却被当记号用的）
const LOOSE: &[env::Mark] = &[
    crate::driver::ENTRY_MARK,
    crate::system::operator::TIP_MARK,
    crate::system::operator::TIP_BACK,
    control::ASK_MARK,
    control::BACK,
    crate::system::identity::BACK,
    crate::system::operator::ASK_MARK,
];

/// **全协议任两枚记号不许撞**：三族的面 × 别族的面 × 上面那几枚散记号，逐对判一次
const _: () = {
    let fams: [&[env::Mark]; 3] = [
        &crate::system::identity::Grant::MARKS,
        &control::Grant::MARKS,
        &crate::system::operator::Grant::MARKS,
    ];
    let mut f = 0;
    while f < fams.len() {
        let a = fams[f];
        let mut i = 0;
        while i < a.len() {
            // 一、与**后面**各族的面（本族内部那一条由 `table!` 自己判）。
            let mut g = f + 1;
            while g < fams.len() {
                let b = fams[g];
                let mut j = 0;
                while j < b.len() {
                    assert!(a[i].get() != b[j].get(), "system: two faces share one mark");
                    j += 1;
                }
                g += 1;
            }
            // 二、与面之外那几枚。
            let mut j = 0;
            while j < LOOSE.len() {
                assert!(
                    a[i].get() != LOOSE[j].get(),
                    "system: a face and a loose mark share one mark"
                );
                j += 1;
            }
            i += 1;
        }
        f += 1;
    }
    // 三、面之外那几枚彼此。
    let mut i = 0;
    while i < LOOSE.len() {
        let mut j = i + 1;
        while j < LOOSE.len() {
            assert!(
                LOOSE[i].get() != LOOSE[j].get(),
                "system: two loose marks share one mark"
            );
            j += 1;
        }
        i += 1;
    }
};
