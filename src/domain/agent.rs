#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Codex,
    Claude,
    Cline,
}
impl Agent {
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cline => "cline",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Cline => "Cline",
        }
    }
    pub fn next_step(self) -> &'static str {
        match self {
            Self::Codex => "Direct local connection; no hooks required.",
            Self::Claude => "Restart Claude Code to load the hooks.",
            Self::Cline => {
                "Turn on Enable Hooks in Cline, then reopen or resume a task. Approvals stay in Cline."
            }
        }
    }
}
