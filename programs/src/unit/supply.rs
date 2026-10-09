/// 实例化一台要多做的一手——通道名。
pub const READY: &str = "ready";
/// 启动关系中的 ready 角色，不是发布权利。
pub const READY_MARK: env::Mark = env::Mark::of(READY);

#[derive(Clone, Copy)]
pub enum Setup {
    Image {
        name: &'static str,
        load: &'static str,
    },
    /// 答得动了：这一台交回一枚刻 `READY` 的孔。
    Ready,
    /// 整机物料：这一台起手要这台机器的全部可领之物。
    Machine {
        /// 收物料那条通道的名字。
        load: &'static str,
        /// "我起完了"那条通道的名字（收方在起手末尾铸一枚刻它的孔）。
        ready: &'static str,
    },
}

impl Setup {
    pub const fn channel(&self) -> &'static str {
        match self {
            Setup::Ready => READY,
            Setup::Machine { load, .. } | Setup::Image { load, .. } => load,
        }
    }

    /// 还有第二条吗——`Machine` 多一格（"我起完了"）。
    pub const fn ready(&self) -> Option<&'static str> {
        match self {
            Setup::Ready | Setup::Image { .. } => None,
            Setup::Machine { ready, .. } => Some(ready),
        }
    }

    pub const fn machine(&self) -> bool {
        matches!(self, Setup::Machine { .. })
    }
}

/// Image recipients explicitly acknowledge consumption before backing pages are released.
pub fn valid_supplies(setups: &[Setup]) -> bool {
    if setups.iter().any(|s| matches!(s, Setup::Image { .. }))
        && !setups.iter().any(|s| matches!(s, Setup::Ready))
    {
        return false;
    }
    let channels = || {
        setups
            .iter()
            .flat_map(|s| [Some(s.channel()), s.ready()].into_iter().flatten())
    };
    for (at, channel) in channels().enumerate() {
        if channel.is_empty()
            || channels()
                .take(at)
                .any(|prior| env::Mark::of(prior) == env::Mark::of(channel))
        {
            return false;
        }
    }
    true
}
