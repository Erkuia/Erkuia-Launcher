#![allow(dead_code)]

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use slint::ComponentHandle;

use crate::{error::ErrorCode, LauncherWindow};

/// Cooperative cancellation shared between the UI thread and background work.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn reset(&self) {
        self.0.store(false, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Sleep in slices so a cancel lands within `SLICE` rather than after the
    /// whole wait. Returns `false` if the wait was cut short.
    pub fn sleep(&self, total: Duration) -> bool {
        const SLICE: Duration = Duration::from_millis(200);

        let mut remaining = total;
        while !remaining.is_zero() {
            if self.is_cancelled() {
                return false;
            }

            let step = remaining.min(SLICE);
            std::thread::sleep(step);
            remaining -= step;
        }

        !self.is_cancelled()
    }
}

/// Ordered phases of the start flow, each owning a slice of the progress bar.
///
/// The spans are contiguous and cover `0.0..=1.0`, so the bar never jumps back
/// or stalls between phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Prepare,
    Auth,
    Java,
    Manifest,
    Download,
    Verify,
    Mods,
    Launch,
}

impl Stage {
    pub const ORDER: [Stage; 8] = [
        Stage::Prepare,
        Stage::Auth,
        Stage::Java,
        Stage::Manifest,
        Stage::Download,
        Stage::Verify,
        Stage::Mods,
        Stage::Launch,
    ];

    pub fn span(self) -> (f32, f32) {
        match self {
            Self::Prepare => (0.00, 0.03),
            Self::Auth => (0.03, 0.08),
            Self::Java => (0.08, 0.25),
            Self::Manifest => (0.25, 0.30),
            Self::Download => (0.30, 0.70),
            Self::Verify => (0.70, 0.85),
            Self::Mods => (0.85, 0.93),
            Self::Launch => (0.93, 1.00),
        }
    }
}

/// Map a stage-local `0.0..=1.0` into the overall bar position.
pub fn overall(stage: Stage, local: f32) -> f32 {
    let (start, end) = stage.span();

    start + (end - start) * local.clamp(0.0, 1.0)
}

/// Handle handed to background work so it can report without knowing about the
/// UI thread.
#[derive(Clone)]
pub struct Reporter {
    app: slint::Weak<LauncherWindow>,
}

impl Reporter {
    pub fn progress(&self, stage: Stage, local: f32, message: impl Into<String>) {
        let message = message.into();
        log::info!("{:?} {message}", stage);

        self.overall(overall(stage, local), message);
    }

    pub fn overall(&self, fraction: f32, message: impl Into<String>) {
        let fraction = fraction.clamp(0.0, 1.0);
        let message = message.into();

        log::info!("{:>3.0}% {message}", fraction * 100.0);

        let _ = self.app.upgrade_in_event_loop(move |app| {
            app.set_progress_indeterminate(false);
            app.set_progress(fraction);
            app.set_status_hint(message.into());
        });
    }

    pub fn waiting(&self, message: impl Into<String>) {
        let message = message.into();
        log::info!("대기 중 · {message}");

        let _ = self.app.upgrade_in_event_loop(move |app| {
            app.set_progress_indeterminate(true);
            app.set_status_hint(message.into());
        });
    }
}

/// Run `work` off the UI thread, driving `busy` / `progress` and routing any
/// failure into the error modal.
pub fn spawn<F>(app: &LauncherWindow, code: ErrorCode, work: F)
where
    F: FnOnce(&Reporter) -> anyhow::Result<()> + Send + 'static,
{
    if app.get_busy() {
        return;
    }

    app.set_busy(true);
    app.set_progress(0.0);
    app.set_progress_indeterminate(false);

    let reporter = Reporter {
        app: app.as_weak(),
    };
    let app = app.as_weak();

    std::thread::spawn(move || {
        let result = work(&reporter);

        let _ = app.upgrade_in_event_loop(move |app| {
            app.set_busy(false);

            app.set_progress_indeterminate(false);

            match result {
                Ok(()) => app.set_progress(1.0),
                Err(error) => {
                    app.set_progress(0.0);
                    crate::show_error(&app, code, &error);
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_is_visible_through_clones() {
        let cancel = Cancel::new();
        let clone = cancel.clone();

        assert!(!clone.is_cancelled());
        cancel.cancel();
        assert!(clone.is_cancelled());
    }

    #[test]
    fn sleeping_returns_early_once_cancelled() {
        let cancel = Cancel::new();
        cancel.cancel();

        let started = std::time::Instant::now();
        let completed = cancel.sleep(Duration::from_secs(5));

        assert!(!completed);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn an_uncancelled_sleep_runs_to_completion() {
        let cancel = Cancel::new();

        assert!(cancel.sleep(Duration::from_millis(50)));
    }

    #[test]
    fn spans_are_contiguous_and_cover_the_whole_bar() {
        let mut cursor = 0.0_f32;

        for stage in Stage::ORDER {
            let (start, end) = stage.span();

            assert!(
                (start - cursor).abs() < f32::EPSILON,
                "{stage:?} starts at {start}, expected {cursor}"
            );
            assert!(end > start, "{stage:?} has an empty span");

            cursor = end;
        }

        assert!((cursor - 1.0).abs() < f32::EPSILON, "bar ends at {cursor}");
    }

    #[test]
    fn maps_local_progress_into_the_stage_span() {
        assert_eq!(overall(Stage::Download, 0.0), 0.30);
        assert_eq!(overall(Stage::Download, 1.0), 0.70);
        assert!((overall(Stage::Download, 0.5) - 0.50).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_local_progress_is_clamped() {
        assert_eq!(overall(Stage::Verify, -5.0), 0.70);
        assert_eq!(overall(Stage::Verify, 42.0), 0.85);
    }

    #[test]
    fn overall_progress_never_moves_backwards() {
        let mut previous = -1.0_f32;

        for stage in Stage::ORDER {
            for step in 0..=10 {
                let value = overall(stage, step as f32 / 10.0);

                assert!(
                    value >= previous,
                    "{stage:?} at {step}/10 went backwards: {value} < {previous}"
                );
                previous = value;
            }
        }

        assert_eq!(previous, 1.0);
    }
}
