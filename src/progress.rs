#[derive(Debug, Clone)]
pub enum InstallEvent {
    Progress {
        stage: InstallStage,
        local_percent: f32,
        message: String,
    },
    Completed,
    Failed {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum InstallStage {
    Prepare,
    Download,
    Verify,
    InstallFiles,
    Shortcuts,
    RegisterUninstaller,
    Finalize,
}

impl InstallStage {
    pub fn range(self) -> ProgressRange {
        match self {
            Self::Prepare => ProgressRange::new(0.0, 5.0),
            Self::Download => ProgressRange::new(5.0, 50.0),
            Self::Verify => ProgressRange::new(50.0, 65.0),
            Self::InstallFiles => ProgressRange::new(65.0, 88.0),
            Self::Shortcuts => ProgressRange::new(88.0, 94.0),
            Self::RegisterUninstaller => ProgressRange::new(94.0, 98.0),
            Self::Finalize => ProgressRange::new(98.0, 100.0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProgressRange {
    start: f32,
    end: f32,
}

impl ProgressRange {
    pub fn new(start: f32, end: f32) -> Self {
        Self { start, end }
    }

    pub fn map(self, local_percent: f32) -> i32 {
        let local = local_percent.clamp(0.0, 100.0) / 100.0;
        (self.start + (self.end - self.start) * local).round() as i32
    }
}

pub fn overall_percent(stage: InstallStage, local_percent: f32) -> i32 {
    stage.range().map(local_percent)
}
