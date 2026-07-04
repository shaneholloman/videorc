use anyhow::{Result, bail};

use crate::protocol::{
    EntitlementCapability, EntitlementLimits, EntitlementSource, EntitlementState, EntitlementTier,
    EntitlementsSnapshot, FeatureId, RecordingEntitlementLimits, StreamingEntitlementLimits,
};

pub const PREMIUM_FEATURES_ENV_VAR: &str = "VIDEORC_PREMIUM_FEATURES";

const ENTITLEMENT_SCHEMA_VERSION: u32 = 1;
const BASIC_MAX_WIDTH: u32 = 1920;
const BASIC_MAX_HEIGHT: u32 = 1080;
const BASIC_MAX_FPS: u32 = 30;
const BASIC_STREAMING_MAX_BITRATE_KBPS: u32 = 6000;
const BASIC_STREAMING_MAX_DESTINATIONS: u32 = 1;
const PREMIUM_RECORDING_MAX_WIDTH: u32 = 3840;
const PREMIUM_RECORDING_MAX_HEIGHT: u32 = 2160;
const PREMIUM_RECORDING_MAX_FPS: u32 = 30;
const PREMIUM_STREAMING_MAX_WIDTH: u32 = 1920;
const PREMIUM_STREAMING_MAX_HEIGHT: u32 = 1080;
const PREMIUM_STREAMING_MAX_FPS: u32 = 30;
const PREMIUM_STREAMING_MAX_BITRATE_KBPS: u32 = 6000;
const PREMIUM_STREAMING_MAX_DESTINATIONS: u32 = 3;
const DEVELOPER_STREAMING_MAX_WIDTH: u32 = 3840;
const DEVELOPER_STREAMING_MAX_HEIGHT: u32 = 2160;
const DEVELOPER_STREAMING_MAX_FPS: u32 = 30;
const DEVELOPER_STREAMING_MAX_BITRATE_KBPS: u32 = 30_000;

const MULTISTREAMING_DISABLED_REASON: &str =
    "Multistreaming requires Videorc Premium. Basic can stream to one destination at HD.";
const CLOUD_AI_DISABLED_REASON: &str = "Cloud AI is a Videorc Premium feature. Set VIDEORC_PREMIUM_FEATURES=1 for local developer testing.";
const DEVELOPER_OVERRIDE_REASON: &str = "Enabled by VIDEORC_PREMIUM_FEATURES=1.";
const DEV_BUILD_OVERRIDE_REASON: &str = "Enabled by Videorc debug/dev backend build.";

// --- Account-hydrated entitlement (multistream premium gate) ------------------
// The signed-in account's server-verified entitlement (from
// /api/ai/capabilities, fetched with the bearer token) is the ONLY way a
// packaged build reaches Premium limits. Fail-closed by construction: absent,
// signed-out, or stale hydration resolves to basic. The staleness ceiling
// bounds how long a cancelled subscription can keep premium after its last
// successful verification.
const ACCOUNT_HYDRATION_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy)]
struct AccountEntitlementHydration {
    is_premium: bool,
    hydrated_at: std::time::Instant,
}

static ACCOUNT_HYDRATION: std::sync::Mutex<Option<AccountEntitlementHydration>> =
    std::sync::Mutex::new(None);

fn set_hydration(
    slot: &std::sync::Mutex<Option<AccountEntitlementHydration>>,
    next: Option<AccountEntitlementHydration>,
) -> bool {
    let mut guard = slot.lock().expect("entitlement hydration lock");
    // "Changed" means the EFFECTIVE outcome moved: hydrated-premium on one
    // side and anything non-premium (hydrated basic OR cleared) on the other.
    let was_premium = guard.is_some_and(|current| current.is_premium);
    let is_premium = next.is_some_and(|hydration| hydration.is_premium);
    *guard = next;
    was_premium != is_premium
}

/// Record the signed-in account's verified entitlement. Returns true when the
/// effective snapshot may have changed (callers emit `entitlements.updated`).
pub fn hydrate_account_entitlements(is_premium: bool) -> bool {
    set_hydration(
        &ACCOUNT_HYDRATION,
        Some(AccountEntitlementHydration {
            is_premium,
            hydrated_at: std::time::Instant::now(),
        }),
    )
}

/// Sign-out / unauthorized: the account no longer vouches for anything.
pub fn clear_account_entitlements() -> bool {
    set_hydration(&ACCOUNT_HYDRATION, None)
}

fn account_hydrated_premium() -> bool {
    ACCOUNT_HYDRATION
        .lock()
        .expect("entitlement hydration lock")
        .is_some_and(|hydration| {
            hydration.is_premium && hydration.hydrated_at.elapsed() <= ACCOUNT_HYDRATION_MAX_AGE
        })
}

pub fn current_entitlements() -> EntitlementsSnapshot {
    let value = std::env::var(PREMIUM_FEATURES_ENV_VAR).ok();
    current_entitlements_resolved(
        value.as_deref(),
        cfg!(debug_assertions),
        account_hydrated_premium(),
    )
}

fn current_entitlements_resolved(
    value: Option<&str>,
    dev_build: bool,
    account_premium: bool,
) -> EntitlementsSnapshot {
    if premium_override_enabled(value) {
        return developer_entitlements(DEVELOPER_OVERRIDE_REASON);
    }

    // Explicit =0/false/off forces basic even in dev builds — the only way to
    // exercise the premium gates (multistream lock etc.) on a dev machine.
    if basic_override_enabled(value) {
        return basic_entitlements();
    }

    if account_premium {
        return premium_entitlements(EntitlementSource::Creem);
    }

    if dev_build {
        return developer_entitlements(DEV_BUILD_OVERRIDE_REASON);
    }

    basic_entitlements()
}

#[cfg(test)]
fn current_entitlements_from_env_value(
    value: Option<&str>,
    dev_build: bool,
) -> EntitlementsSnapshot {
    current_entitlements_resolved(value, dev_build, false)
}

#[cfg(test)]
pub fn entitlements_from_env_value(value: Option<&str>) -> EntitlementsSnapshot {
    if premium_override_enabled(value) {
        return developer_entitlements(DEVELOPER_OVERRIDE_REASON);
    }

    basic_entitlements()
}

pub fn basic_entitlements() -> EntitlementsSnapshot {
    EntitlementsSnapshot {
        schema_version: ENTITLEMENT_SCHEMA_VERSION,
        tier: EntitlementTier::Basic,
        source: EntitlementSource::LocalDefault,
        capabilities: vec![
            EntitlementCapability {
                feature_id: FeatureId::LocalRecording,
                state: EntitlementState::Enabled,
                reason: None,
            },
            EntitlementCapability {
                feature_id: FeatureId::Livestreaming,
                state: EntitlementState::Enabled,
                reason: None,
            },
            EntitlementCapability {
                feature_id: FeatureId::Multistreaming,
                state: EntitlementState::Disabled,
                reason: Some(MULTISTREAMING_DISABLED_REASON.to_string()),
            },
            EntitlementCapability {
                feature_id: FeatureId::CloudAi,
                state: EntitlementState::Disabled,
                reason: Some(CLOUD_AI_DISABLED_REASON.to_string()),
            },
        ],
        limits: basic_limits(),
        checked_at: None,
        expires_at: None,
    }
}

pub fn premium_entitlements(source: EntitlementSource) -> EntitlementsSnapshot {
    EntitlementsSnapshot {
        schema_version: ENTITLEMENT_SCHEMA_VERSION,
        tier: EntitlementTier::Premium,
        source,
        capabilities: enabled_capabilities(EntitlementState::Enabled, None),
        limits: premium_limits(),
        checked_at: None,
        expires_at: None,
    }
}

fn developer_entitlements(reason: &str) -> EntitlementsSnapshot {
    let mut snapshot = premium_entitlements(EntitlementSource::EnvOverride);
    snapshot.tier = EntitlementTier::Developer;
    snapshot.limits = developer_limits();
    for capability in &mut snapshot.capabilities {
        capability.state = EntitlementState::DeveloperOverride;
        capability.reason = Some(reason.to_string());
    }
    snapshot
}

fn enabled_capabilities(
    state: EntitlementState,
    reason: Option<&str>,
) -> Vec<EntitlementCapability> {
    [
        FeatureId::LocalRecording,
        FeatureId::Livestreaming,
        FeatureId::Multistreaming,
        FeatureId::CloudAi,
    ]
    .into_iter()
    .map(|feature_id| EntitlementCapability {
        feature_id,
        state,
        reason: reason.map(str::to_string),
    })
    .collect()
}

fn basic_limits() -> EntitlementLimits {
    EntitlementLimits {
        recording: RecordingEntitlementLimits {
            max_width: BASIC_MAX_WIDTH,
            max_height: BASIC_MAX_HEIGHT,
            max_fps: BASIC_MAX_FPS,
            max_bitrate_kbps: None,
        },
        streaming: StreamingEntitlementLimits {
            max_width: BASIC_MAX_WIDTH,
            max_height: BASIC_MAX_HEIGHT,
            max_fps: BASIC_MAX_FPS,
            max_bitrate_kbps: BASIC_STREAMING_MAX_BITRATE_KBPS,
            max_destinations: BASIC_STREAMING_MAX_DESTINATIONS,
        },
    }
}

fn premium_limits() -> EntitlementLimits {
    EntitlementLimits {
        recording: RecordingEntitlementLimits {
            max_width: PREMIUM_RECORDING_MAX_WIDTH,
            max_height: PREMIUM_RECORDING_MAX_HEIGHT,
            max_fps: PREMIUM_RECORDING_MAX_FPS,
            max_bitrate_kbps: None,
        },
        streaming: StreamingEntitlementLimits {
            max_width: PREMIUM_STREAMING_MAX_WIDTH,
            max_height: PREMIUM_STREAMING_MAX_HEIGHT,
            max_fps: PREMIUM_STREAMING_MAX_FPS,
            max_bitrate_kbps: PREMIUM_STREAMING_MAX_BITRATE_KBPS,
            max_destinations: PREMIUM_STREAMING_MAX_DESTINATIONS,
        },
    }
}

fn developer_limits() -> EntitlementLimits {
    EntitlementLimits {
        recording: RecordingEntitlementLimits {
            max_width: PREMIUM_RECORDING_MAX_WIDTH,
            max_height: PREMIUM_RECORDING_MAX_HEIGHT,
            max_fps: PREMIUM_RECORDING_MAX_FPS,
            max_bitrate_kbps: None,
        },
        streaming: StreamingEntitlementLimits {
            max_width: DEVELOPER_STREAMING_MAX_WIDTH,
            max_height: DEVELOPER_STREAMING_MAX_HEIGHT,
            max_fps: DEVELOPER_STREAMING_MAX_FPS,
            max_bitrate_kbps: DEVELOPER_STREAMING_MAX_BITRATE_KBPS,
            max_destinations: PREMIUM_STREAMING_MAX_DESTINATIONS,
        },
    }
}

fn capability(
    snapshot: &EntitlementsSnapshot,
    feature_id: FeatureId,
) -> Option<&EntitlementCapability> {
    snapshot
        .capabilities
        .iter()
        .find(|capability| capability.feature_id == feature_id)
}

#[cfg(test)]
fn feature_entitled(snapshot: &EntitlementsSnapshot, feature_id: FeatureId) -> bool {
    capability(snapshot, feature_id)
        .map(|capability| capability.state != EntitlementState::Disabled)
        .unwrap_or(false)
}

pub fn require_feature(snapshot: &EntitlementsSnapshot, feature_id: FeatureId) -> Result<()> {
    let Some(capability) = capability(snapshot, feature_id) else {
        bail!("Feature entitlement is missing from the backend capability model.");
    };

    if capability.state == EntitlementState::Disabled {
        bail!(
            "{}",
            capability
                .reason
                .as_deref()
                .unwrap_or("This Videorc feature is not enabled.")
        );
    }

    Ok(())
}

fn basic_override_enabled(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase()),
        Some(value) if matches!(value.as_str(), "0" | "false" | "no" | "off" | "basic")
    )
}

fn premium_override_enabled(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase()),
        Some(value)
            if matches!(
                value.as_str(),
                "1" | "true" | "yes" | "on" | "premium" | "developer" | "all"
            )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entitlement_default_snapshot_is_basic_with_hd_recording_and_one_hd_livestream() {
        let snapshot = entitlements_from_env_value(None);

        assert_eq!(snapshot.schema_version, ENTITLEMENT_SCHEMA_VERSION);
        assert_eq!(snapshot.tier, EntitlementTier::Basic);
        assert_eq!(snapshot.source, EntitlementSource::LocalDefault);
        assert!(feature_entitled(&snapshot, FeatureId::LocalRecording));
        assert!(feature_entitled(&snapshot, FeatureId::Livestreaming));
        assert!(!feature_entitled(&snapshot, FeatureId::Multistreaming));
        assert!(!feature_entitled(&snapshot, FeatureId::CloudAi));
        assert_eq!(snapshot.limits.recording.max_width, 1920);
        assert_eq!(snapshot.limits.recording.max_height, 1080);
        assert_eq!(snapshot.limits.recording.max_fps, 30);
        assert_eq!(snapshot.limits.streaming.max_width, 1920);
        assert_eq!(snapshot.limits.streaming.max_height, 1080);
        assert_eq!(snapshot.limits.streaming.max_fps, 30);
        assert_eq!(snapshot.limits.streaming.max_bitrate_kbps, 6000);
        assert_eq!(snapshot.limits.streaming.max_destinations, 1);
    }

    #[test]
    fn premium_snapshot_enables_multistreaming_and_cloud_ai() {
        let snapshot = premium_entitlements(EntitlementSource::Creem);

        assert_eq!(snapshot.schema_version, ENTITLEMENT_SCHEMA_VERSION);
        assert_eq!(snapshot.tier, EntitlementTier::Premium);
        assert_eq!(snapshot.source, EntitlementSource::Creem);
        assert!(feature_entitled(&snapshot, FeatureId::LocalRecording));
        assert!(feature_entitled(&snapshot, FeatureId::Livestreaming));
        assert!(feature_entitled(&snapshot, FeatureId::Multistreaming));
        assert!(feature_entitled(&snapshot, FeatureId::CloudAi));
        assert_eq!(snapshot.limits.recording.max_width, 3840);
        assert_eq!(snapshot.limits.recording.max_height, 2160);
        assert_eq!(snapshot.limits.streaming.max_destinations, 3);
    }

    #[test]
    fn entitlement_env_override_enables_premium_features_for_development() {
        let snapshot = entitlements_from_env_value(Some("1"));

        assert_eq!(snapshot.tier, EntitlementTier::Developer);
        assert_eq!(snapshot.source, EntitlementSource::EnvOverride);
        assert!(feature_entitled(&snapshot, FeatureId::Livestreaming));
        assert!(feature_entitled(&snapshot, FeatureId::Multistreaming));
        assert!(feature_entitled(&snapshot, FeatureId::CloudAi));
        assert_eq!(
            capability(&snapshot, FeatureId::Livestreaming)
                .expect("livestreaming capability")
                .state,
            EntitlementState::DeveloperOverride
        );
        assert_eq!(snapshot.limits.streaming.max_width, 3840);
        assert_eq!(snapshot.limits.streaming.max_height, 2160);
        assert_eq!(snapshot.limits.streaming.max_bitrate_kbps, 30_000);
    }

    #[test]
    fn current_entitlements_enable_developer_features_for_dev_builds_without_env() {
        let snapshot = current_entitlements_from_env_value(None, true);

        assert_eq!(snapshot.tier, EntitlementTier::Developer);
        assert_eq!(snapshot.source, EntitlementSource::EnvOverride);
        assert!(feature_entitled(&snapshot, FeatureId::Multistreaming));
        assert!(feature_entitled(&snapshot, FeatureId::CloudAi));
        assert_eq!(snapshot.limits.streaming.max_destinations, 3);
        assert_eq!(
            capability(&snapshot, FeatureId::Multistreaming)
                .expect("multistreaming capability")
                .reason
                .as_deref(),
            Some(DEV_BUILD_OVERRIDE_REASON)
        );
    }

    #[test]
    fn account_hydration_resolution_is_fail_closed() {
        // No hydration: release stays basic, dev stays premium.
        assert_eq!(
            current_entitlements_resolved(None, false, false).tier,
            EntitlementTier::Basic
        );
        // Hydrated premium account unlocks premium limits in release builds.
        let hydrated = current_entitlements_resolved(None, false, true);
        assert_eq!(hydrated.tier, EntitlementTier::Premium);
        assert_eq!(hydrated.source, EntitlementSource::Creem);
        assert!(hydrated.limits.streaming.max_destinations > 1);
        // Env basic override beats everything except the premium override —
        // the only way to test the gates on a dev machine.
        assert_eq!(
            current_entitlements_resolved(Some("0"), true, true).tier,
            EntitlementTier::Basic
        );
        assert_eq!(
            current_entitlements_resolved(Some("off"), true, true).tier,
            EntitlementTier::Basic
        );
    }

    #[test]
    fn account_hydration_state_machine_tracks_changes() {
        // A LOCAL slot: tests must never mutate the process-global hydration —
        // other tests read current_entitlements() in parallel threads.
        let slot: std::sync::Mutex<Option<AccountEntitlementHydration>> =
            std::sync::Mutex::new(None);
        let hydration = |premium: bool| AccountEntitlementHydration {
            is_premium: premium,
            hydrated_at: std::time::Instant::now(),
        };
        assert!(
            set_hydration(&slot, Some(hydration(true))),
            "basic->premium changes"
        );
        assert!(
            !set_hydration(&slot, Some(hydration(true))),
            "premium->premium is a no-op"
        );
        assert!(
            set_hydration(&slot, Some(hydration(false))),
            "premium->basic changes"
        );
        assert!(
            !set_hydration(&slot, None),
            "clearing non-premium is a no-op"
        );
        set_hydration(&slot, Some(hydration(true)));
        assert!(set_hydration(&slot, None), "sign-out drops premium");
    }

    #[test]
    fn current_entitlements_keep_release_builds_basic_without_env() {
        let snapshot = current_entitlements_from_env_value(None, false);

        assert_eq!(snapshot.tier, EntitlementTier::Basic);
        assert_eq!(snapshot.source, EntitlementSource::LocalDefault);
        assert!(!feature_entitled(&snapshot, FeatureId::Multistreaming));
    }

    #[test]
    fn entitlement_env_override_accepts_explicit_truthy_values_only() {
        assert!(feature_entitled(
            &entitlements_from_env_value(Some("developer")),
            FeatureId::CloudAi
        ));
        assert!(!feature_entitled(
            &entitlements_from_env_value(Some("0")),
            FeatureId::CloudAi
        ));
        assert!(!feature_entitled(
            &entitlements_from_env_value(Some("")),
            FeatureId::CloudAi
        ));
    }

    #[test]
    fn entitlement_snapshot_uses_protocol_field_names() {
        let snapshot = entitlements_from_env_value(Some("true"));
        let value = serde_json::to_value(snapshot).unwrap();

        assert_eq!(value["schemaVersion"], json!(1));
        assert_eq!(value["tier"], json!("developer"));
        assert_eq!(value["source"], json!("env-override"));
        assert_eq!(
            value["capabilities"][0]["featureId"],
            json!("local-recording")
        );
        assert_eq!(
            value["capabilities"][1]["state"],
            json!("developer-override")
        );
        assert_eq!(value["limits"]["streaming"]["maxDestinations"], json!(3));
    }

    #[test]
    fn entitlement_require_feature_returns_disabled_reason() {
        let snapshot = entitlements_from_env_value(None);
        let error = require_feature(&snapshot, FeatureId::CloudAi)
            .expect_err("cloud AI should be gated in Basic mode");

        assert!(error.to_string().contains("Premium"));
    }
}
