use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail, ensure};
use block2::RcBlock;
use objc2_core_foundation::CFRetained;
use objc2_core_media::{CMSampleBuffer, CMTime, kCMTimeInvalid, kCMVideoCodecType_H264};
use objc2_core_video::{CVPixelBufferGetHeight, CVPixelBufferGetWidth};
use objc2_video_toolbox::{VTCompressionSession, VTEncodeInfoFlags};

use crate::metal_compositor::MetalCompositorTargetPixelBuffer;

const NO_ERR: OSStatus = 0;
type OSStatus = i32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoToolboxH264ProbeResult {
    pub width: usize,
    pub height: usize,
    pub create_status: OSStatus,
    pub prepare_status: OSStatus,
    pub encode_status: Option<OSStatus>,
    pub complete_status: Option<OSStatus>,
    pub encode_info_flags: Option<u32>,
    pub callback_count: usize,
    pub callback_status: Option<OSStatus>,
    pub callback_info_flags: Option<u32>,
    pub sample_buffer_count: usize,
    pub sample_total_size_bytes: usize,
    pub block_buffer_data_bytes: usize,
    pub frame_dropped: bool,
    pub iosurface_backed: bool,
}

impl VideoToolboxH264ProbeResult {
    fn prepared(width: usize, height: usize, prepare_status: OSStatus) -> Self {
        Self {
            width,
            height,
            create_status: NO_ERR,
            prepare_status,
            encode_status: None,
            complete_status: None,
            encode_info_flags: None,
            callback_count: 0,
            callback_status: None,
            callback_info_flags: None,
            sample_buffer_count: 0,
            sample_total_size_bytes: 0,
            block_buffer_data_bytes: 0,
            frame_dropped: false,
            iosurface_backed: false,
        }
    }
}

#[derive(Debug, Default)]
struct EncodeCallbackState {
    callback_count: usize,
    status: Option<OSStatus>,
    info_flags: Option<u32>,
    sample_buffer_count: usize,
    sample_total_size_bytes: usize,
    block_buffer_data_bytes: usize,
    frame_dropped: bool,
}

pub struct VideoToolboxH264Session {
    session: CFRetained<VTCompressionSession>,
    width: usize,
    height: usize,
}

impl VideoToolboxH264Session {
    pub fn new(width: usize, height: usize) -> Result<Self> {
        ensure!(width > 0, "VideoToolbox session width must be non-zero");
        ensure!(height > 0, "VideoToolbox session height must be non-zero");
        ensure!(
            width <= i32::MAX as usize && height <= i32::MAX as usize,
            "VideoToolbox session dimensions exceed i32 range: {width}x{height}"
        );

        let mut raw_session: *mut VTCompressionSession = ptr::null_mut();
        let status = unsafe {
            VTCompressionSession::create(
                None,
                width as i32,
                height as i32,
                kCMVideoCodecType_H264,
                None,
                None,
                None,
                None,
                ptr::null_mut(),
                NonNull::from(&mut raw_session),
            )
        };
        if status != NO_ERR {
            bail!("VTCompressionSessionCreate(H.264 {width}x{height}) failed with {status}");
        }
        let raw_session =
            NonNull::new(raw_session).context("VTCompressionSessionCreate returned null")?;
        let session = unsafe { CFRetained::from_raw(raw_session) };
        Ok(Self {
            session,
            width,
            height,
        })
    }

    pub fn prepare(&self) -> Result<VideoToolboxH264ProbeResult> {
        let status = unsafe { self.session.prepare_to_encode_frames() };
        if status != NO_ERR {
            bail!(
                "VTCompressionSessionPrepareToEncodeFrames(H.264 {}x{}) failed with {status}",
                self.width,
                self.height
            );
        }
        Ok(VideoToolboxH264ProbeResult::prepared(
            self.width,
            self.height,
            status,
        ))
    }

    pub fn encode_retained_target(
        &self,
        target: &MetalCompositorTargetPixelBuffer,
    ) -> Result<VideoToolboxH264ProbeResult> {
        let pixel_buffer = target.pixel_buffer();
        let width = CVPixelBufferGetWidth(pixel_buffer);
        let height = CVPixelBufferGetHeight(pixel_buffer);
        ensure!(
            width == self.width && height == self.height,
            "target dimensions {width}x{height} do not match VideoToolbox session {}x{}",
            self.width,
            self.height
        );
        let iosurface_backed = target.has_iosurface();
        ensure!(
            iosurface_backed,
            "retained Metal target is not IOSurface-backed"
        );

        let state = Arc::new(Mutex::new(EncodeCallbackState::default()));
        let callback_state = state.clone();
        let output_handler: RcBlock<dyn Fn(OSStatus, VTEncodeInfoFlags, *mut CMSampleBuffer)> =
            RcBlock::new(
                move |status: OSStatus,
                      info_flags: VTEncodeInfoFlags,
                      sample_buffer: *mut CMSampleBuffer| {
                    let mut state = callback_state
                        .lock()
                        .expect("encode callback state poisoned");
                    state.callback_count += 1;
                    state.status = Some(status);
                    state.info_flags = Some(info_flags.0);
                    if !sample_buffer.is_null() {
                        state.sample_buffer_count += 1;
                        let sample_buffer = unsafe { &*sample_buffer };
                        state.sample_total_size_bytes +=
                            unsafe { sample_buffer.total_sample_size() };
                        if let Some(data_buffer) = unsafe { sample_buffer.data_buffer() } {
                            state.block_buffer_data_bytes += unsafe { data_buffer.data_length() };
                        }
                    }
                    state.frame_dropped |= info_flags.contains(VTEncodeInfoFlags::FrameDropped);
                },
            );

        let mut encode_info_flags = VTEncodeInfoFlags::empty();
        let presentation_time = unsafe { CMTime::new(0, 60) };
        let duration = unsafe { CMTime::new(1, 60) };
        let encode_status = unsafe {
            self.session.encode_frame_with_output_handler(
                pixel_buffer,
                presentation_time,
                duration,
                None,
                &mut encode_info_flags,
                RcBlock::as_ptr(&output_handler),
            )
        };
        if encode_status != NO_ERR {
            bail!(
                "VTCompressionSessionEncodeFrameWithOutputHandler(H.264 {}x{}) failed with {encode_status}",
                self.width,
                self.height
            );
        }
        let complete_status = unsafe { self.session.complete_frames(kCMTimeInvalid) };
        if complete_status != NO_ERR {
            bail!(
                "VTCompressionSessionCompleteFrames(H.264 {}x{}) failed with {complete_status}",
                self.width,
                self.height
            );
        }

        let state = state.lock().expect("encode callback state poisoned");
        Ok(VideoToolboxH264ProbeResult {
            width,
            height,
            create_status: NO_ERR,
            prepare_status: NO_ERR,
            encode_status: Some(encode_status),
            complete_status: Some(complete_status),
            encode_info_flags: Some(encode_info_flags.0),
            callback_count: state.callback_count,
            callback_status: state.status,
            callback_info_flags: state.info_flags,
            sample_buffer_count: state.sample_buffer_count,
            sample_total_size_bytes: state.sample_total_size_bytes,
            block_buffer_data_bytes: state.block_buffer_data_bytes,
            frame_dropped: state.frame_dropped
                || encode_info_flags.contains(VTEncodeInfoFlags::FrameDropped),
            iosurface_backed,
        })
    }
}

impl Drop for VideoToolboxH264Session {
    fn drop(&mut self) {
        unsafe { self.session.invalidate() };
    }
}

#[allow(dead_code)]
pub fn probe_h264_session_ready(
    width: usize,
    height: usize,
) -> Result<VideoToolboxH264ProbeResult> {
    let session = VideoToolboxH264Session::new(width, height)?;
    session.prepare()
}

#[allow(dead_code)]
pub fn probe_h264_encode_retained_target(
    target: &MetalCompositorTargetPixelBuffer,
) -> Result<VideoToolboxH264ProbeResult> {
    let session = VideoToolboxH264Session::new(target.width(), target.height())?;
    let mut result = session.prepare()?;
    result.iosurface_backed = target.has_iosurface();
    let encode_result = session.encode_retained_target(target)?;
    Ok(VideoToolboxH264ProbeResult {
        prepare_status: result.prepare_status,
        iosurface_backed: result.iosurface_backed,
        ..encode_result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metal_compositor::{GpuSource, MetalSceneCompositor};

    #[test]
    fn video_toolbox_h264_session_prepares_or_skips() {
        match probe_h264_session_ready(64, 64) {
            Ok(result) => {
                assert_eq!(result.width, 64);
                assert_eq!(result.height, 64);
                assert_eq!(result.create_status, NO_ERR);
                assert_eq!(result.prepare_status, NO_ERR);
            }
            Err(error) => {
                eprintln!("skipping: VideoToolbox H.264 session unavailable: {error:#}");
            }
        }
    }

    #[test]
    fn video_toolbox_accepts_retained_metal_target_or_skips() {
        let Some(mut compositor) = MetalSceneCompositor::new() else {
            eprintln!("skipping: no Metal device available in this environment");
            return;
        };
        let red = [0u8, 0, 255, 255];
        let sources = [GpuSource {
            bgra: &red,
            width: 1,
            height: 1,
            dest: [0.0, 0.0, 1.0, 1.0],
            crop: [0.0; 4],
            mirror: false,
            circle: false,
        }];
        compositor
            .compose_bgra(64, 64, [0.0, 0.0, 0.0, 1.0], &sources)
            .expect("compose retained Metal target");
        let Some(target) = compositor.latest_target_pixel_buffer() else {
            eprintln!("skipping: IOSurface-backed Metal target unavailable");
            return;
        };

        match probe_h264_encode_retained_target(&target) {
            Ok(result) => {
                assert_eq!(result.width, 64);
                assert_eq!(result.height, 64);
                assert_eq!(result.create_status, NO_ERR);
                assert_eq!(result.prepare_status, NO_ERR);
                assert_eq!(result.encode_status, Some(NO_ERR));
                assert_eq!(result.complete_status, Some(NO_ERR));
                assert!(result.iosurface_backed);
                assert!(
                    !result.frame_dropped,
                    "VideoToolbox dropped the retained target frame: {result:?}"
                );
                assert!(
                    result.callback_count > 0,
                    "VideoToolbox did not invoke the encode callback: {result:?}"
                );
                assert_eq!(result.callback_status, Some(NO_ERR));
                assert!(
                    result.sample_buffer_count > 0,
                    "VideoToolbox accepted the frame but returned no sample buffer: {result:?}"
                );
                assert!(
                    result.sample_total_size_bytes > 0,
                    "VideoToolbox sample buffer had no encoded sample bytes: {result:?}"
                );
                assert!(
                    result.block_buffer_data_bytes > 0,
                    "VideoToolbox sample buffer had no accessible CMBlockBuffer bytes: {result:?}"
                );
            }
            Err(error) => {
                eprintln!(
                    "skipping: VideoToolbox could not encode retained Metal target: {error:#}"
                );
            }
        }
    }
}
