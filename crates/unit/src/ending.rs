/// 谁结束它——这一台的寿命由谁定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    /// 我收它：只有被收才走。
    Resident,
    Transient,
    /// 有人叫它走：终止词从外面来。
    Told,
}
