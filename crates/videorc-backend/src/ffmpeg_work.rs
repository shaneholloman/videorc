use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenanceDeferral {
    CaptureActive,
    FinalizingActive,
    MaintenanceRunning,
}

impl MaintenanceDeferral {
    pub fn message(self) -> &'static str {
        match self {
            MaintenanceDeferral::CaptureActive => {
                "Deferred while recording or streaming is active."
            }
            MaintenanceDeferral::FinalizingActive => "Deferred while the recording is finalizing.",
            MaintenanceDeferral::MaintenanceRunning => {
                "Deferred while another recording maintenance job is running."
            }
        }
    }
}

#[derive(Debug, Default)]
struct FfmpegWorkState {
    capture_waiting: usize,
    capture_active: bool,
    finalizing_active: bool,
    maintenance_running: bool,
}

#[derive(Debug, Default)]
pub struct FfmpegWorkCoordinator {
    state: Mutex<FfmpegWorkState>,
    notify: Notify,
}

impl FfmpegWorkCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn begin_capture_when_available(self: &Arc<Self>) -> CapturePermit {
        let mut waiting_registered = false;
        loop {
            let notified = {
                let mut state = self.state.lock().expect("ffmpeg work state poisoned");
                if !state.maintenance_running && !state.finalizing_active {
                    if waiting_registered {
                        state.capture_waiting = state.capture_waiting.saturating_sub(1);
                    }
                    state.capture_active = true;
                    return CapturePermit {
                        coordinator: self.clone(),
                    };
                }
                if !waiting_registered {
                    state.capture_waiting += 1;
                    waiting_registered = true;
                }
                self.notify.notified()
            };
            notified.await;
        }
    }

    pub fn begin_finalizing(self: &Arc<Self>) -> FinalizingPermit {
        {
            let mut state = self.state.lock().expect("ffmpeg work state poisoned");
            state.finalizing_active = true;
        }
        self.notify.notify_waiters();
        FinalizingPermit {
            coordinator: self.clone(),
        }
    }

    pub fn try_begin_maintenance(
        self: &Arc<Self>,
    ) -> Result<MaintenancePermit, MaintenanceDeferral> {
        let mut state = self.state.lock().expect("ffmpeg work state poisoned");
        if state.capture_active {
            return Err(MaintenanceDeferral::CaptureActive);
        }
        if state.capture_waiting > 0 {
            return Err(MaintenanceDeferral::CaptureActive);
        }
        if state.finalizing_active {
            return Err(MaintenanceDeferral::FinalizingActive);
        }
        if state.maintenance_running {
            return Err(MaintenanceDeferral::MaintenanceRunning);
        }
        state.maintenance_running = true;
        Ok(MaintenancePermit {
            coordinator: self.clone(),
        })
    }

    pub async fn begin_maintenance_when_idle(self: &Arc<Self>) -> MaintenancePermit {
        loop {
            match self.try_begin_maintenance() {
                Ok(permit) => return permit,
                Err(_) => self.notify.notified().await,
            }
        }
    }

    #[cfg(test)]
    pub fn current_deferral(&self) -> Option<MaintenanceDeferral> {
        let state = self.state.lock().expect("ffmpeg work state poisoned");
        if state.capture_active || state.capture_waiting > 0 {
            Some(MaintenanceDeferral::CaptureActive)
        } else if state.finalizing_active {
            Some(MaintenanceDeferral::FinalizingActive)
        } else if state.maintenance_running {
            Some(MaintenanceDeferral::MaintenanceRunning)
        } else {
            None
        }
    }

    fn end_capture(&self) {
        {
            let mut state = self.state.lock().expect("ffmpeg work state poisoned");
            state.capture_active = false;
        }
        self.notify.notify_waiters();
    }

    fn end_finalizing(&self) {
        {
            let mut state = self.state.lock().expect("ffmpeg work state poisoned");
            state.finalizing_active = false;
        }
        self.notify.notify_waiters();
    }

    fn end_maintenance(&self) {
        {
            let mut state = self.state.lock().expect("ffmpeg work state poisoned");
            state.maintenance_running = false;
        }
        self.notify.notify_waiters();
    }
}

#[derive(Debug)]
pub struct CapturePermit {
    coordinator: Arc<FfmpegWorkCoordinator>,
}

impl Drop for CapturePermit {
    fn drop(&mut self) {
        self.coordinator.end_capture();
    }
}

#[derive(Debug)]
pub struct FinalizingPermit {
    coordinator: Arc<FfmpegWorkCoordinator>,
}

impl Drop for FinalizingPermit {
    fn drop(&mut self) {
        self.coordinator.end_finalizing();
    }
}

#[derive(Debug)]
pub struct MaintenancePermit {
    coordinator: Arc<FfmpegWorkCoordinator>,
}

impl Drop for MaintenancePermit {
    fn drop(&mut self) {
        self.coordinator.end_maintenance();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn maintenance_is_deferred_while_capture_is_active() {
        let coordinator = Arc::new(FfmpegWorkCoordinator::new());
        let capture = coordinator.begin_capture_when_available().await;

        assert_eq!(
            coordinator.try_begin_maintenance().unwrap_err(),
            MaintenanceDeferral::CaptureActive
        );

        drop(capture);
        assert!(coordinator.try_begin_maintenance().is_ok());
    }

    #[tokio::test]
    async fn capture_waits_for_active_maintenance_to_finish() {
        let coordinator = Arc::new(FfmpegWorkCoordinator::new());
        let maintenance = coordinator.try_begin_maintenance().unwrap();

        assert_eq!(
            coordinator.current_deferral(),
            Some(MaintenanceDeferral::MaintenanceRunning)
        );

        drop(maintenance);
        let capture = coordinator.begin_capture_when_available().await;
        assert_eq!(
            coordinator.try_begin_maintenance().unwrap_err(),
            MaintenanceDeferral::CaptureActive
        );
        drop(capture);
    }

    #[tokio::test]
    async fn capture_waits_for_finalization_to_finish() {
        let coordinator = Arc::new(FfmpegWorkCoordinator::new());
        let finalizing = coordinator.begin_finalizing();

        assert_eq!(
            coordinator.current_deferral(),
            Some(MaintenanceDeferral::FinalizingActive)
        );

        drop(finalizing);
        let capture = coordinator.begin_capture_when_available().await;
        assert_eq!(
            coordinator.try_begin_maintenance().unwrap_err(),
            MaintenanceDeferral::CaptureActive
        );
        drop(capture);
    }

    #[tokio::test]
    async fn waiting_capture_defers_pending_maintenance() {
        let coordinator = Arc::new(FfmpegWorkCoordinator::new());
        let finalizing = coordinator.begin_finalizing();
        let waiting_capture = tokio::spawn({
            let coordinator = coordinator.clone();
            async move { coordinator.begin_capture_when_available().await }
        });

        tokio::task::yield_now().await;
        assert_eq!(
            coordinator.try_begin_maintenance().unwrap_err(),
            MaintenanceDeferral::CaptureActive
        );

        drop(finalizing);
        let capture = waiting_capture.await.unwrap();
        assert_eq!(
            coordinator.try_begin_maintenance().unwrap_err(),
            MaintenanceDeferral::CaptureActive
        );
        drop(capture);
    }
}
