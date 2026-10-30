#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub key_code: u32,
    pub modifiers: u32,
    pub key_name: String,
}
