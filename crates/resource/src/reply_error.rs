//! 一次交互回信端的失败词。

use env::HoleFail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyError {
    Mail(HoleFail),
    WrongSource,
}
