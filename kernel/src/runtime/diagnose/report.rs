//! report —— 诊断报告核心：信息收集模块化，一次印发全部信息。
//!
//! 领域意象：组件投稿（段落 + 行）→ 成册（[`Report::seal`] 打戳）→ 印发。

use alloc::{string::String, vec::Vec};
use serde::Serialize;

use crate::machine;
use crate::runtime::chrono::clock;

/// 报告：全量诊断信息的唯一容器。
#[derive(Serialize, Default)]
pub struct Report {
    /// 成册戳 [hart, ticks]（[`Report::seal`] 写入；wire 上一个数组不打两个键）。
    seal: (usize, u64),
    /// 全部段落。
    pub paras: Vec<Paragraph>,
}

/// 段落：段名（= 档位）+ 标题 + 行集合（可空槽数组；首行恒为表头）。
#[derive(Serialize)]
pub struct Paragraph {
    pub name: &'static str,
    pub title: Option<String>,
    pub items: Vec<Vec<Option<String>>>,
}

impl Report {
    /// 开一段并取引用（append；借用在语句/作用域结束时释放 = 收段）。
    /// 新段空 `items`；`name` 即档位（同报告内不强制唯一）。
    pub fn paragraph(&mut self, name: &'static str, title: Option<String>) -> &mut Paragraph {
        self.paras.push(Paragraph {
            name,
            title,
            items: Vec::new(),
        });
        self.paras.last_mut().expect("just pushed")
    }

    /// 成册：打戳（hart, ticks），可写借用转只读引用。允许空报告；
    /// 此后仅经 `&Report` 读；重刊须重新借得 `&mut` 后调 [`Report::clear`]。
    pub fn seal(&mut self) -> &Self {
        self.seal = (machine::hart_id(), clock::now().as_ticks());
        self
    }

    /// 清空重刊：清空全部段落（打戳位待下次 [`Report::seal`] 重写）。
    #[allow(unused)]
    pub fn clear(&mut self) {
        self.paras.clear();
    }
}
