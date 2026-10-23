pub(super) enum LaunchReply {
    Pending,
    Started,
    Abort { trusted: bool },
}
