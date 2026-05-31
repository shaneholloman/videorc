use crate::protocol::{Device, DeviceKind, DeviceStatus};

const NATIVE_CAMERA_PREFIX: &str = "camera:avfoundation-native:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCameraDevices {
    pub devices: Vec<Device>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraFormatSummary {
    pub width: u32,
    pub height: u32,
    pub min_fps: f64,
    pub max_fps: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraFormatChoice {
    pub format: CameraFormatSummary,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCameraPermission {
    Authorized,
    NotDetermined,
    Denied,
    Restricted,
    Unknown,
}

pub fn list_native_cameras() -> NativeCameraDevices {
    #[cfg(target_os = "macos")]
    {
        macos::list_native_cameras()
    }

    #[cfg(not(target_os = "macos"))]
    {
        NativeCameraDevices {
            devices: Vec::new(),
            warnings: vec![
                "Native AVFoundation camera discovery is only available on macOS.".to_string(),
            ],
        }
    }
}

pub fn native_camera_name_for_id(camera_id: &str) -> Option<String> {
    let unique_id = parse_native_camera_id(camera_id)?;

    #[cfg(target_os = "macos")]
    {
        macos::camera_name_for_unique_id(&unique_id)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = unique_id;
        None
    }
}

pub fn native_camera_device_id(unique_id: &str) -> String {
    format!("{NATIVE_CAMERA_PREFIX}{}", encode_hex(unique_id.as_bytes()))
}

pub fn parse_native_camera_id(id: &str) -> Option<String> {
    let encoded = id.strip_prefix(NATIVE_CAMERA_PREFIX)?;
    let bytes = decode_hex(encoded)?;
    String::from_utf8(bytes).ok()
}

pub fn camera_permission_status(permission: NativeCameraPermission) -> DeviceStatus {
    match permission {
        NativeCameraPermission::Authorized => DeviceStatus::Available,
        NativeCameraPermission::NotDetermined
        | NativeCameraPermission::Denied
        | NativeCameraPermission::Restricted => DeviceStatus::PermissionRequired,
        NativeCameraPermission::Unknown => DeviceStatus::Unavailable,
    }
}

pub fn choose_camera_format(
    formats: &[CameraFormatSummary],
    target_width: u32,
    target_height: u32,
    target_fps: u32,
) -> Option<CameraFormatChoice> {
    let supports_target = |format: &&CameraFormatSummary| {
        format.width == target_width
            && format.height == target_height
            && format.min_fps <= f64::from(target_fps)
            && format.max_fps >= f64::from(target_fps)
    };

    if let Some(format) = formats.iter().find(supports_target) {
        return Some(CameraFormatChoice {
            format: (*format).clone(),
            fallback_reason: None,
        });
    }

    let target_pixels = u64::from(target_width) * u64::from(target_height);
    formats
        .iter()
        .filter(|format| format.max_fps >= f64::from(target_fps))
        .min_by_key(|format| {
            let pixels = u64::from(format.width) * u64::from(format.height);
            pixels.abs_diff(target_pixels)
        })
        .or_else(|| formats.iter().max_by_key(|format| format.width * format.height))
        .map(|format| CameraFormatChoice {
            format: format.clone(),
            fallback_reason: Some(format!(
                "Requested {target_width}x{target_height}@{target_fps} was not available; selected {}x{} up to {:.0} fps.",
                format.width, format.height, format.max_fps
            )),
        })
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }

    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_value(chunk[0])?;
            let low = hex_value(chunk[1])?;
            Some((high << 4) | low)
        })
        .collect()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeVideo};
    use objc2_core_media::CMVideoFormatDescriptionGetDimensions;
    use objc2_foundation::NSString;

    pub fn list_native_cameras() -> NativeCameraDevices {
        let Some(video_media_type) = video_media_type() else {
            return NativeCameraDevices {
                devices: vec![unavailable_camera(
                    "camera:avfoundation-native-media-type-missing",
                    "AVFoundation video media type is unavailable.",
                )],
                warnings: vec!["AVFoundation video media type is unavailable.".to_string()],
            };
        };

        let permission = native_camera_permission();
        let status = camera_permission_status(permission);
        #[allow(deprecated)]
        let devices = unsafe { AVCaptureDevice::devicesWithMediaType(video_media_type) };
        let mut camera_devices = Vec::new();

        for index in 0..devices.count() {
            let camera = devices.objectAtIndex(index);
            let unique_id = unsafe { camera.uniqueID() };
            let unique_id =
                ns_string_to_string(&unique_id).unwrap_or_else(|| format!("unknown-{index}"));
            let name = unsafe { camera.localizedName() };
            let name =
                ns_string_to_string(&name).unwrap_or_else(|| format!("Camera {}", index + 1));
            let formats = camera_formats(&camera);
            let active_format = active_camera_format_detail(&camera);
            let permission_detail = camera_permission_detail(permission);
            let detail = match (active_format, permission_detail) {
                (Some(active_format), Some(permission_detail)) => {
                    format!("{permission_detail} {active_format}")
                }
                (Some(active_format), None) => active_format,
                (None, Some(permission_detail)) => permission_detail.to_string(),
                (None, None) => {
                    "Native AVFoundation camera. Recording currently uses the FFmpeg fallback bridge."
                        .to_string()
                }
            };

            let choice = choose_camera_format(&formats, 1920, 1080, 30);
            let detail = if let Some(reason) = choice.and_then(|choice| choice.fallback_reason) {
                format!("{detail} {reason}")
            } else {
                detail
            };

            camera_devices.push(Device {
                id: native_camera_device_id(&unique_id),
                name,
                kind: DeviceKind::Camera,
                status: status.clone(),
                detail: Some(detail),
            });
        }

        if camera_devices.is_empty() {
            camera_devices.push(unavailable_camera(
                "camera:avfoundation-native-missing",
                if status == DeviceStatus::PermissionRequired {
                    "AVFoundation did not return cameras. Camera permission may be missing."
                } else {
                    "AVFoundation did not return any video cameras."
                },
            ));
        }

        NativeCameraDevices {
            devices: camera_devices,
            warnings: camera_permission_warning(permission).into_iter().collect(),
        }
    }

    pub fn camera_name_for_unique_id(unique_id: &str) -> Option<String> {
        let unique_id = NSString::from_str(unique_id);
        let camera = unsafe { AVCaptureDevice::deviceWithUniqueID(&unique_id) }?;
        let name = unsafe { camera.localizedName() };
        ns_string_to_string(&name)
    }

    fn native_camera_permission() -> NativeCameraPermission {
        let Some(video_media_type) = video_media_type() else {
            return NativeCameraPermission::Unknown;
        };
        match unsafe { AVCaptureDevice::authorizationStatusForMediaType(video_media_type) } {
            status if status == AVAuthorizationStatus::Authorized => {
                NativeCameraPermission::Authorized
            }
            status if status == AVAuthorizationStatus::NotDetermined => {
                NativeCameraPermission::NotDetermined
            }
            status if status == AVAuthorizationStatus::Denied => NativeCameraPermission::Denied,
            status if status == AVAuthorizationStatus::Restricted => {
                NativeCameraPermission::Restricted
            }
            _ => NativeCameraPermission::Unknown,
        }
    }

    fn video_media_type() -> Option<&'static objc2_av_foundation::AVMediaType> {
        unsafe { AVMediaTypeVideo }
    }

    fn camera_permission_detail(permission: NativeCameraPermission) -> Option<&'static str> {
        match permission {
            NativeCameraPermission::Authorized => None,
            NativeCameraPermission::NotDetermined => {
                Some("Camera permission has not been granted yet.")
            }
            NativeCameraPermission::Denied => Some("Camera permission is denied."),
            NativeCameraPermission::Restricted => Some("Camera permission is restricted by macOS."),
            NativeCameraPermission::Unknown => Some("Camera permission state is unknown."),
        }
    }

    fn camera_permission_warning(permission: NativeCameraPermission) -> Option<String> {
        match permission {
            NativeCameraPermission::Authorized => None,
            NativeCameraPermission::NotDetermined => Some(
                "Camera permission has not been granted yet. Open Camera privacy settings if preview shows black frames."
                    .to_string(),
            ),
            NativeCameraPermission::Denied | NativeCameraPermission::Restricted => Some(
                "Camera permission is blocked. Open macOS Camera privacy settings and enable Videorc or the development shell."
                    .to_string(),
            ),
            NativeCameraPermission::Unknown => {
                Some("Could not determine Camera permission state.".to_string())
            }
        }
    }

    fn camera_formats(camera: &AVCaptureDevice) -> Vec<CameraFormatSummary> {
        let formats = unsafe { camera.formats() };
        let mut summaries = Vec::new();

        for index in 0..formats.count() {
            let format = formats.objectAtIndex(index);
            let description = unsafe { format.formatDescription() };
            let dimensions = unsafe { CMVideoFormatDescriptionGetDimensions(&description) };
            let ranges = unsafe { format.videoSupportedFrameRateRanges() };

            for range_index in 0..ranges.count() {
                let range = ranges.objectAtIndex(range_index);
                summaries.push(CameraFormatSummary {
                    width: dimensions.width.max(0) as u32,
                    height: dimensions.height.max(0) as u32,
                    min_fps: unsafe { range.minFrameRate() },
                    max_fps: unsafe { range.maxFrameRate() },
                });
            }
        }

        summaries
    }

    fn active_camera_format_detail(camera: &AVCaptureDevice) -> Option<String> {
        let active_format = unsafe { camera.activeFormat() };
        let description = unsafe { active_format.formatDescription() };
        let dimensions = unsafe { CMVideoFormatDescriptionGetDimensions(&description) };
        let ranges = unsafe { active_format.videoSupportedFrameRateRanges() };
        let max_fps = max_frame_rate(&ranges);
        let width = dimensions.width.max(0);
        let height = dimensions.height.max(0);

        if width == 0 || height == 0 {
            return None;
        }

        Some(match max_fps {
            Some(max_fps) => format!(
                "Native AVFoundation camera active format: {width}x{height} up to {max_fps:.0} fps. Recording currently uses the FFmpeg fallback bridge."
            ),
            None => format!(
                "Native AVFoundation camera active format: {width}x{height}. Recording currently uses the FFmpeg fallback bridge."
            ),
        })
    }

    fn max_frame_rate(
        ranges: &objc2_foundation::NSArray<objc2_av_foundation::AVFrameRateRange>,
    ) -> Option<f64> {
        let mut max_fps: Option<f64> = None;
        for index in 0..ranges.count() {
            let range = ranges.objectAtIndex(index);
            let fps = unsafe { range.maxFrameRate() };
            max_fps = Some(max_fps.map_or(fps, |current| current.max(fps)));
        }
        max_fps
    }

    fn unavailable_camera(id: &str, detail: &str) -> Device {
        Device {
            id: id.to_string(),
            name: "Camera".to_string(),
            kind: DeviceKind::Camera,
            status: DeviceStatus::Unavailable,
            detail: Some(detail.to_string()),
        }
    }

    fn ns_string_to_string(value: &NSString) -> Option<String> {
        let value = value.to_string();
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_camera_ids_round_trip_unique_ids() {
        let unique_id = "AppleCamera-0x8020000005ac8514";
        let device_id = native_camera_device_id(unique_id);

        assert_eq!(
            parse_native_camera_id(&device_id).as_deref(),
            Some(unique_id)
        );
        assert_eq!(parse_native_camera_id("camera:avfoundation:0"), None);
        assert_eq!(
            parse_native_camera_id("camera:avfoundation-native:not-hex"),
            None
        );
    }

    #[test]
    fn maps_camera_permission_to_device_status() {
        assert_eq!(
            camera_permission_status(NativeCameraPermission::Authorized),
            DeviceStatus::Available
        );
        assert_eq!(
            camera_permission_status(NativeCameraPermission::Denied),
            DeviceStatus::PermissionRequired
        );
        assert_eq!(
            camera_permission_status(NativeCameraPermission::Unknown),
            DeviceStatus::Unavailable
        );
    }

    #[test]
    fn chooses_exact_camera_format_when_available() {
        let formats = vec![
            CameraFormatSummary {
                width: 1280,
                height: 720,
                min_fps: 1.0,
                max_fps: 60.0,
            },
            CameraFormatSummary {
                width: 1920,
                height: 1080,
                min_fps: 1.0,
                max_fps: 30.0,
            },
        ];

        let choice = choose_camera_format(&formats, 1920, 1080, 30).unwrap();

        assert_eq!(choice.format.width, 1920);
        assert_eq!(choice.format.height, 1080);
        assert_eq!(choice.fallback_reason, None);
    }

    #[test]
    fn chooses_clear_camera_format_fallback() {
        let formats = vec![CameraFormatSummary {
            width: 1280,
            height: 720,
            min_fps: 1.0,
            max_fps: 60.0,
        }];

        let choice = choose_camera_format(&formats, 1920, 1080, 30).unwrap();

        assert_eq!(choice.format.width, 1280);
        assert!(choice.fallback_reason.unwrap().contains("not available"));
    }
}
