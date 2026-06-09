use crate::protocol::{PreviewSurfaceBacking, PreviewSurfaceBounds, PreviewTransport};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePreviewHostBounds {
    pub screen_x: f64,
    pub screen_y: f64,
    pub width: f64,
    pub height: f64,
    pub scale_factor: f64,
    #[serde(default)]
    pub screen_height: Option<f64>,
    // Visible clip rect in the same screen coordinate space; absent = fully visible.
    #[serde(default)]
    pub clip_x: Option<f64>,
    #[serde(default)]
    pub clip_y: Option<f64>,
    #[serde(default)]
    pub clip_width: Option<f64>,
    #[serde(default)]
    pub clip_height: Option<f64>,
    // False = hide the surface entirely (slot scrolled away / document hidden).
    #[serde(default)]
    pub visible: Option<bool>,
}

impl NativePreviewHostBounds {
    #[allow(dead_code)]
    pub fn from_surface_bounds(bounds: &PreviewSurfaceBounds) -> Self {
        Self {
            screen_x: bounds.screen_x,
            screen_y: bounds.screen_y,
            width: bounds.width.max(1.0),
            height: bounds.height.max(1.0),
            scale_factor: bounds.scale_factor.max(1.0),
            screen_height: bounds.screen_height,
            clip_x: bounds.clip_x,
            clip_y: bounds.clip_y,
            clip_width: bounds.clip_width.map(|width| width.max(0.0)),
            clip_height: bounds.clip_height.map(|height| height.max(0.0)),
            visible: bounds.visible,
        }
    }

    pub fn drawable_size(self) -> (f64, f64) {
        (
            self.width * self.scale_factor,
            self.height * self.scale_factor,
        )
    }

    pub fn appkit_frame(self) -> (f64, f64, f64, f64) {
        let appkit_y = self
            .screen_height
            .filter(|screen_height| screen_height.is_finite())
            .map(|screen_height| screen_height - self.screen_y - self.height)
            .unwrap_or(self.screen_y);
        (self.screen_x, appkit_y, self.width, self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativePreviewHostCommandKind {
    Create,
    UpdateBounds,
    Destroy,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePreviewHostCommand {
    pub kind: NativePreviewHostCommandKind,
    pub bounds: Option<NativePreviewHostBounds>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativePreviewHostActivation {
    pub transport: PreviewTransport,
    pub backing: PreviewSurfaceBacking,
    pub presented_frame_id: u64,
    pub frame_polling_suppressed: bool,
    pub source_pixels_present: bool,
    pub message: Option<String>,
}

impl NativePreviewHostActivation {
    pub fn cametal_layer_presented(presented_frame_id: u64) -> Self {
        Self {
            transport: PreviewTransport::NativeSurface,
            backing: PreviewSurfaceBacking::CaMetalLayer,
            presented_frame_id,
            frame_polling_suppressed: true,
            source_pixels_present: true,
            message: Some(
                "Native CAMetalLayer preview surface is presenting compositor output.".to_string(),
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePreviewHostPresentFailure {
    MissingOverlay,
    IosurfaceImportFailed,
    DrawableUnavailable,
    CommandBufferUnavailable,
    EncodeFailed,
}

impl NativePreviewHostPresentFailure {
    #[allow(dead_code)]
    pub fn reason(self) -> &'static str {
        match self {
            Self::MissingOverlay => "missing-overlay",
            Self::IosurfaceImportFailed => "iosurface-import-failed",
            Self::DrawableUnavailable => "drawable-unavailable",
            Self::CommandBufferUnavailable => "command-buffer-unavailable",
            Self::EncodeFailed => "encode-failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NativePreviewHostLifecycleUpdate {
    pub command: Option<NativePreviewHostCommand>,
    pub activation: Option<NativePreviewHostActivation>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NativePreviewHostLifecycle {
    last_command: Option<NativePreviewHostCommandKind>,
    bounds: Option<NativePreviewHostBounds>,
}

impl NativePreviewHostLifecycle {
    pub fn create(&mut self, bounds: &PreviewSurfaceBounds) -> NativePreviewHostLifecycleUpdate {
        self.last_command = Some(NativePreviewHostCommandKind::Create);
        let bounds = NativePreviewHostBounds::from_surface_bounds(bounds);
        self.bounds = Some(bounds);
        NativePreviewHostLifecycleUpdate {
            command: Some(NativePreviewHostCommand {
                kind: NativePreviewHostCommandKind::Create,
                bounds: Some(bounds),
            }),
            activation: None,
        }
    }

    pub fn update_bounds(
        &mut self,
        bounds: &PreviewSurfaceBounds,
    ) -> NativePreviewHostLifecycleUpdate {
        self.last_command = Some(NativePreviewHostCommandKind::UpdateBounds);
        let bounds = NativePreviewHostBounds::from_surface_bounds(bounds);
        self.bounds = Some(bounds);
        NativePreviewHostLifecycleUpdate {
            command: Some(NativePreviewHostCommand {
                kind: NativePreviewHostCommandKind::UpdateBounds,
                bounds: Some(bounds),
            }),
            activation: None,
        }
    }

    pub fn destroy(&mut self) -> NativePreviewHostLifecycleUpdate {
        self.last_command = Some(NativePreviewHostCommandKind::Destroy);
        self.bounds = None;
        NativePreviewHostLifecycleUpdate {
            command: Some(NativePreviewHostCommand {
                kind: NativePreviewHostCommandKind::Destroy,
                bounds: None,
            }),
            activation: None,
        }
    }

    #[cfg(test)]
    pub fn last_command_kind(&self) -> Option<NativePreviewHostCommandKind> {
        self.last_command
    }

    #[cfg(test)]
    pub fn bounds(&self) -> Option<NativePreviewHostBounds> {
        self.bounds
    }
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod macos {
    use objc2::rc::Retained;
    use objc2::{ClassType, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSColor, NSFloatingWindowLevel, NSView, NSWindow,
        NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use objc2_quartz_core::{CALayer, CAMetalLayer};

    use super::{
        NativePreviewHostActivation, NativePreviewHostBounds, NativePreviewHostCommand,
        NativePreviewHostCommandKind, NativePreviewHostPresentFailure,
    };
    use crate::metal_compositor::{
        MetalImportedIosurfaceTexture, MetalPreviewPresentFailure, MetalPreviewPresenter,
        MetalSceneCompositor, make_preview_layer,
    };

    impl From<MetalPreviewPresentFailure> for NativePreviewHostPresentFailure {
        fn from(failure: MetalPreviewPresentFailure) -> Self {
            match failure {
                MetalPreviewPresentFailure::IosurfaceImportFailed => Self::IosurfaceImportFailed,
                MetalPreviewPresentFailure::DrawableUnavailable => Self::DrawableUnavailable,
                MetalPreviewPresentFailure::CommandBufferUnavailable => {
                    Self::CommandBufferUnavailable
                }
                MetalPreviewPresentFailure::EncodeFailed => Self::EncodeFailed,
            }
        }
    }

    #[derive(Debug)]
    pub struct NativePreviewLayerHost {
        view: Retained<NSView>,
        layer: Retained<CAMetalLayer>,
        bounds: NativePreviewHostBounds,
    }

    impl NativePreviewLayerHost {
        pub fn new(
            presenter: &MetalPreviewPresenter,
            bounds: NativePreviewHostBounds,
            mtm: MainThreadMarker,
        ) -> Self {
            let (drawable_width, drawable_height) = bounds.drawable_size();
            let layer = make_preview_layer(presenter.device(), drawable_width, drawable_height);
            let view = NSView::initWithFrame(NSView::alloc(mtm), view_frame(bounds));
            view.setWantsLayer(true);
            let ca_layer: &CALayer = layer.as_super();
            view.setLayer(Some(ca_layer));
            Self {
                view,
                layer,
                bounds,
            }
        }

        pub fn view(&self) -> &NSView {
            &self.view
        }

        pub fn layer(&self) -> &CAMetalLayer {
            &self.layer
        }

        pub fn bounds(&self) -> NativePreviewHostBounds {
            self.bounds
        }

        pub fn set_bounds(&mut self, bounds: NativePreviewHostBounds) {
            let (drawable_width, drawable_height) = bounds.drawable_size();
            self.layer.setDrawableSize(objc2_core_foundation::CGSize {
                width: drawable_width,
                height: drawable_height,
            });
            self.view.setFrame(view_frame(bounds));
            self.bounds = bounds;
        }
    }

    #[derive(Debug)]
    pub struct NativePreviewOverlayHost {
        window: Retained<NSWindow>,
        layer_host: NativePreviewLayerHost,
    }

    impl NativePreviewOverlayHost {
        pub fn new(
            presenter: &MetalPreviewPresenter,
            bounds: NativePreviewHostBounds,
            mtm: MainThreadMarker,
        ) -> Self {
            let layer_host = NativePreviewLayerHost::new(presenter, bounds, mtm);
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    window_frame(bounds),
                    NSWindowStyleMask::Borderless,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            window.setContentView(Some(layer_host.view()));
            window.setOpaque(false);
            window.setBackgroundColor(Some(&NSColor::clearColor()));
            window.setIgnoresMouseEvents(false);
            window.setMovable(true);
            window.setMovableByWindowBackground(true);
            // The overlay has to sit above Studio's own content to be visible. Keep it hidden
            // outside Videorc and out of the window cycle, while allowing users to drag the
            // borderless preview away if it lands over controls.
            window.setLevel(NSFloatingWindowLevel);
            window.setHasShadow(false);
            window.setHidesOnDeactivate(true);
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::Transient
                    | NSWindowCollectionBehavior::IgnoresCycle
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            unsafe {
                window.setReleasedWhenClosed(false);
            }
            Self { window, layer_host }
        }

        pub fn window(&self) -> &NSWindow {
            &self.window
        }

        pub fn layer_host(&self) -> &NativePreviewLayerHost {
            &self.layer_host
        }

        pub fn layer(&self) -> &CAMetalLayer {
            self.layer_host.layer()
        }

        pub fn set_bounds(&mut self, bounds: NativePreviewHostBounds) {
            self.layer_host.set_bounds(bounds);
            self.window.setFrame_display(window_frame(bounds), true);
        }

        pub fn show(&self) {
            self.window.orderFrontRegardless();
        }

        pub fn hide(&self) {
            self.window.orderOut(None);
        }
    }

    /// Main-thread presenter runtime for the real native preview path.
    ///
    /// This owns AppKit objects and therefore must only be driven from code that holds a
    /// `MainThreadMarker`. Creating/updating the overlay is not enough to claim OBS-native
    /// preview; activation is returned only after the compositor target is actually
    /// presented into the `CAMetalLayer`.
    #[derive(Debug)]
    pub struct NativePreviewPresenterRunner {
        presenter: MetalPreviewPresenter,
        overlay: Option<NativePreviewOverlayHost>,
    }

    impl NativePreviewPresenterRunner {
        pub fn new(compositor: &MetalSceneCompositor) -> Option<Self> {
            Some(Self {
                presenter: compositor.make_preview_presenter()?,
                overlay: None,
            })
        }

        pub fn apply_command(&mut self, command: NativePreviewHostCommand, mtm: MainThreadMarker) {
            apply_overlay_command(&self.presenter, &mut self.overlay, command, mtm);
        }

        pub fn present_latest(
            &self,
            compositor: &MetalSceneCompositor,
            presented_frame_id: u64,
        ) -> Option<NativePreviewHostActivation> {
            let overlay = self.overlay.as_ref()?;
            compositor
                .present_latest_to_layer(&self.presenter, overlay.layer())
                .then(|| NativePreviewHostActivation::cametal_layer_presented(presented_frame_id))
        }

        pub fn has_overlay(&self) -> bool {
            self.overlay.is_some()
        }
    }

    /// Main-thread native preview runtime for imported compositor IOSurface handoffs.
    ///
    /// This is the host shape needed by an Electron native addon or backend AppKit
    /// overlay that receives `metalTargetIosurfaceId` from compositor status. It still
    /// only returns activation after a real layer present succeeds.
    #[derive(Debug)]
    pub struct NativePreviewIosurfacePresenterRunner {
        presenter: MetalPreviewPresenter,
        overlay: Option<NativePreviewOverlayHost>,
        cached_texture: Option<MetalImportedIosurfaceTexture>,
    }

    impl NativePreviewIosurfacePresenterRunner {
        pub fn new() -> Option<Self> {
            Some(Self {
                presenter: MetalPreviewPresenter::new_default()?,
                overlay: None,
                cached_texture: None,
            })
        }

        pub fn apply_command(&mut self, command: NativePreviewHostCommand, mtm: MainThreadMarker) {
            self.cached_texture = None;
            apply_overlay_command(&self.presenter, &mut self.overlay, command, mtm);
        }

        pub fn present_iosurface(
            &mut self,
            iosurface_id: u32,
            width: usize,
            height: usize,
            presented_frame_id: u64,
        ) -> Option<NativePreviewHostActivation> {
            self.try_present_iosurface(iosurface_id, width, height, presented_frame_id)
                .ok()
        }

        pub fn try_present_iosurface(
            &mut self,
            iosurface_id: u32,
            width: usize,
            height: usize,
            presented_frame_id: u64,
        ) -> Result<NativePreviewHostActivation, NativePreviewHostPresentFailure> {
            if self.overlay.is_none() {
                return Err(NativePreviewHostPresentFailure::MissingOverlay);
            }
            let needs_import = self
                .cached_texture
                .as_ref()
                .is_none_or(|texture| !texture.matches(iosurface_id, width, height));
            if needs_import {
                let imported = self
                    .presenter
                    .import_iosurface_texture_handle(iosurface_id, width, height)
                    .ok_or(NativePreviewHostPresentFailure::IosurfaceImportFailed)?;
                self.cached_texture = Some(imported);
            }
            let overlay = self
                .overlay
                .as_ref()
                .ok_or(NativePreviewHostPresentFailure::MissingOverlay)?;
            let imported = self
                .cached_texture
                .as_ref()
                .ok_or(NativePreviewHostPresentFailure::IosurfaceImportFailed)?;
            self.presenter
                .try_present_imported_iosurface_to_layer(overlay.layer(), imported)
                .map_err(NativePreviewHostPresentFailure::from)?;
            Ok(NativePreviewHostActivation::cametal_layer_presented(
                presented_frame_id,
            ))
        }

        pub fn has_overlay(&self) -> bool {
            self.overlay.is_some()
        }
    }

    fn apply_overlay_command(
        presenter: &MetalPreviewPresenter,
        overlay: &mut Option<NativePreviewOverlayHost>,
        command: NativePreviewHostCommand,
        mtm: MainThreadMarker,
    ) {
        match command.kind {
            NativePreviewHostCommandKind::Create | NativePreviewHostCommandKind::UpdateBounds => {
                let Some(bounds) = command.bounds else {
                    return;
                };
                match overlay.as_mut() {
                    Some(overlay) => {
                        overlay.set_bounds(bounds);
                        overlay.show();
                    }
                    None => {
                        let next_overlay = NativePreviewOverlayHost::new(presenter, bounds, mtm);
                        next_overlay.show();
                        *overlay = Some(next_overlay);
                    }
                }
            }
            NativePreviewHostCommandKind::Destroy => {
                if let Some(overlay) = overlay.take() {
                    overlay.hide();
                }
            }
        }
    }

    fn view_frame(bounds: NativePreviewHostBounds) -> NSRect {
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(bounds.width, bounds.height),
        )
    }

    fn window_frame(bounds: NativePreviewHostBounds) -> NSRect {
        let (x, y, width, height) = bounds.appkit_frame();
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }
}

#[cfg(target_os = "macos")]
#[allow(unused_imports)]
pub use macos::{
    NativePreviewIosurfacePresenterRunner, NativePreviewLayerHost, NativePreviewOverlayHost,
    NativePreviewPresenterRunner,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_bounds_clamp_to_visible_drawable_size() {
        let bounds = PreviewSurfaceBounds {
            screen_x: 10.0,
            screen_y: 20.0,
            width: 0.0,
            height: 450.0,
            scale_factor: 2.0,
            screen_height: Some(1000.0),
            ..Default::default()
        };

        let host_bounds = NativePreviewHostBounds::from_surface_bounds(&bounds);

        assert_eq!(host_bounds.width, 1.0);
        assert_eq!(host_bounds.height, 450.0);
        assert_eq!(host_bounds.drawable_size(), (2.0, 900.0));
        assert_eq!(host_bounds.appkit_frame(), (10.0, 530.0, 1.0, 450.0));
    }

    #[test]
    fn host_bounds_carry_clip_and_visibility() {
        let bounds = PreviewSurfaceBounds {
            screen_x: 10.0,
            screen_y: 20.0,
            width: 640.0,
            height: 360.0,
            scale_factor: 2.0,
            screen_height: Some(1000.0),
            clip_x: Some(10.0),
            clip_y: Some(120.0),
            clip_width: Some(640.0),
            clip_height: Some(-4.0),
            visible: Some(true),
        };

        let host_bounds = NativePreviewHostBounds::from_surface_bounds(&bounds);

        assert_eq!(host_bounds.clip_x, Some(10.0));
        assert_eq!(host_bounds.clip_y, Some(120.0));
        assert_eq!(host_bounds.clip_width, Some(640.0));
        // Negative clip sizes clamp to an empty clip instead of poisoning AppKit math.
        assert_eq!(host_bounds.clip_height, Some(0.0));
        assert_eq!(host_bounds.visible, Some(true));
    }

    #[test]
    fn host_bounds_fall_back_to_reported_y_without_screen_height() {
        let host_bounds = NativePreviewHostBounds {
            screen_x: 10.0,
            screen_y: 20.0,
            width: 640.0,
            height: 360.0,
            scale_factor: 1.0,
            screen_height: None,
            ..Default::default()
        };

        assert_eq!(host_bounds.appkit_frame(), (10.0, 20.0, 640.0, 360.0));
    }

    #[test]
    fn lifecycle_records_create_update_destroy_commands() {
        let mut lifecycle = NativePreviewHostLifecycle::default();
        let create_bounds = PreviewSurfaceBounds {
            screen_x: 10.0,
            screen_y: 20.0,
            width: 640.0,
            height: 360.0,
            scale_factor: 2.0,
            screen_height: Some(1000.0),
            ..Default::default()
        };

        let create_update = lifecycle.create(&create_bounds);

        assert_eq!(create_update.activation, None);
        assert_eq!(
            create_update.command,
            Some(NativePreviewHostCommand {
                kind: NativePreviewHostCommandKind::Create,
                bounds: Some(NativePreviewHostBounds {
                    screen_x: 10.0,
                    screen_y: 20.0,
                    width: 640.0,
                    height: 360.0,
                    scale_factor: 2.0,
                    screen_height: Some(1000.0),
                    ..Default::default()
                }),
            })
        );
        assert_eq!(
            lifecycle.last_command_kind(),
            Some(NativePreviewHostCommandKind::Create)
        );
        assert_eq!(
            lifecycle
                .bounds()
                .map(NativePreviewHostBounds::appkit_frame),
            Some((10.0, 620.0, 640.0, 360.0))
        );

        let update_bounds = PreviewSurfaceBounds {
            width: 800.0,
            height: 450.0,
            ..create_bounds
        };

        let bounds_update = lifecycle.update_bounds(&update_bounds);

        assert_eq!(bounds_update.activation, None);
        assert_eq!(
            bounds_update.command,
            Some(NativePreviewHostCommand {
                kind: NativePreviewHostCommandKind::UpdateBounds,
                bounds: Some(NativePreviewHostBounds {
                    screen_x: 10.0,
                    screen_y: 20.0,
                    width: 800.0,
                    height: 450.0,
                    scale_factor: 2.0,
                    screen_height: Some(1000.0),
                    ..Default::default()
                }),
            })
        );
        assert_eq!(
            lifecycle.last_command_kind(),
            Some(NativePreviewHostCommandKind::UpdateBounds)
        );
        assert_eq!(
            lifecycle
                .bounds()
                .map(NativePreviewHostBounds::drawable_size),
            Some((1600.0, 900.0))
        );

        let destroy_update = lifecycle.destroy();

        assert_eq!(destroy_update.activation, None);
        assert_eq!(
            destroy_update.command,
            Some(NativePreviewHostCommand {
                kind: NativePreviewHostCommandKind::Destroy,
                bounds: None,
            })
        );
        assert_eq!(
            lifecycle.last_command_kind(),
            Some(NativePreviewHostCommandKind::Destroy)
        );
        assert_eq!(lifecycle.bounds(), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn iosurface_runner_without_overlay_never_claims_activation() {
        let Some(mut runner) = NativePreviewIosurfacePresenterRunner::new() else {
            eprintln!("skipping: Metal preview presenter unavailable");
            return;
        };

        assert!(!runner.has_overlay());
        assert_eq!(runner.present_iosurface(1, 8, 4, 12), None);
        assert_eq!(
            runner
                .try_present_iosurface(1, 8, 4, 12)
                .expect_err("missing overlay must not activate")
                .reason(),
            "missing-overlay"
        );
    }
}
