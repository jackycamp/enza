#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffMode {
    SideBySide,
    Inline,
}

impl DiffMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::SideBySide => Self::Inline,
            Self::Inline => Self::SideBySide,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusPane {
    Files,
    Main,
}

impl FocusPane {
    pub fn next(self) -> Self {
        match self {
            Self::Files => Self::Main,
            Self::Main => Self::Files,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Files => Self::Main,
            Self::Main => Self::Files,
        }
    }
}

#[derive(Debug)]
pub struct GlobalState {
    pub running: bool,
    pub mode: DiffMode,
    pub focus: FocusPane,
    pub debug_pane_open: bool,
}
