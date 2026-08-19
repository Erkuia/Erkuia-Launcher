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
    pub fn manifest_weight(self, weights: &ProgressWeights) -> &ProgressWeight {
        match self {
            Self::Prepare => &weights.prepare,
            Self::Download => &weights.download,
            Self::Verify => &weights.verify,
            Self::InstallFiles => &weights.install_files,
            Self::Shortcuts => &weights.shortcuts,
            Self::RegisterUninstaller => &weights.register_uninstaller,
            Self::Finalize => &weights.finalize,
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

pub fn overall_percent_with_weights(
    weights: &ProgressWeights,
    stage: InstallStage,
    local_percent: f32,
) -> i32 {
    let weight = stage.manifest_weight(weights);
    ProgressRange::new(weight.start, weight.end).map(local_percent)
}
use crate::manifest::{ProgressWeight, ProgressWeights};
