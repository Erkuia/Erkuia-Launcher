#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Welcome,
    InstallPath,
    Installing,
    Complete,
    Error,
}

impl Step {
    pub fn index(self) -> i32 {
        match self {
            Self::Welcome => 0,
            Self::InstallPath => 1,
            Self::Installing => 2,
            Self::Complete => 3,
            Self::Error => 4,
        }
    }

    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::InstallPath,
            2 => Self::Installing,
            3 => Self::Complete,
            4 => Self::Error,
            _ => Self::Welcome,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Welcome => Self::InstallPath,
            Self::InstallPath => Self::Installing,
            Self::Installing => Self::Complete,
            Self::Complete => Self::Complete,
            Self::Error => Self::InstallPath,
        }
    }
}
