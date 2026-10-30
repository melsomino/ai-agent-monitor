#[derive(Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub connected: bool,
    pub enabled: bool,
    pub paired: bool,
    pub text: String,
    pub link: Option<String>,
    pub queued: usize,
    pub delivered: usize,
    pub snapshot_pending: usize,
    pub suppressed: usize,
    pub command_menu_ready: bool,
}
