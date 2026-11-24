//! Graphics preferences: how frames reach the screen, and which wgpu backend a start prefers.
//!
//! Saved locally as `graphics.prefs` (this machine's GPU and display, not something to sync).
//! The surface fields apply on the next paint; a backend choice needs a restart.

use crate::store::Store;
use eframe::egui_wgpu::SurfaceConfig;
use eframe::wgpu::{Backends, PresentMode};
use serde::{Deserialize, Serialize};

/// How presents wait on the display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplaySync {
    /// Prefer no vsync wait (Immediate → Mailbox → Fifo). Snappy mouse; may tear.
    #[default]
    LowLatency,
    /// Wait for the refresh (Fifo). Smoother on some setups; more input lag.
    Vsync,
    /// Never wait when the driver allows it. Most tearing, least wait.
    Uncapped,
}

impl DisplaySync {
    /// Whether the simple VSync switch is on.
    pub fn vsync_on(self) -> bool {
        matches!(self, Self::Vsync)
    }

    /// Whether Advanced "ignore refresh rate" is on (only meaningful with VSync off).
    pub fn uncapped(self) -> bool {
        matches!(self, Self::Uncapped)
    }

    /// Build from the simple VSync switch and the Advanced uncapped switch.
    pub fn from_switches(vsync: bool, uncapped: bool) -> Self {
        if vsync {
            Self::Vsync
        } else if uncapped {
            Self::Uncapped
        } else {
            Self::LowLatency
        }
    }

    fn present_mode(self) -> PresentMode {
        match self {
            Self::LowLatency => PresentMode::AutoNoVsync,
            Self::Vsync => PresentMode::AutoVsync,
            Self::Uncapped => PresentMode::Immediate,
        }
    }
}

/// How many frames the GPU may queue ahead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FrameQueue {
    /// One frame in flight: least input lag.
    #[default]
    One,
    /// Two frames: higher throughput, more lag (egui-wgpu's old default).
    Two,
}

impl FrameQueue {
    pub const ALL: [Self; 2] = [Self::One, Self::Two];

    /// Plain-language label for Settings.
    pub fn title(self) -> &'static str {
        match self {
            Self::One => crate::i18n::tr!("Snappy"),
            Self::Two => crate::i18n::tr!("Smoother"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::One => crate::i18n::tr!("Keeps the mouse feeling immediate."),
            Self::Two => {
                crate::i18n::tr!(
                    "Can look smoother when the GPU is busy, but the mouse may feel heavier."
                )
            }
        }
    }

    fn latency(self) -> u32 {
        match self {
            Self::One => 1,
            Self::Two => 2,
        }
    }
}

/// Which wgpu backends a start should try first. Changing this needs a restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackendChoice {
    /// The platform ladder (`dx12` → `gl` → `vulkan` on Windows, and so on).
    #[default]
    Automatic,
    #[serde(rename = "dx12")]
    Dx12,
    OpenGl,
    Vulkan,
    Metal,
}

impl BackendChoice {
    /// Choices this build can start with.
    pub fn selectable() -> &'static [Self] {
        #[cfg(windows)]
        {
            &[Self::Automatic, Self::Dx12, Self::OpenGl, Self::Vulkan]
        }
        #[cfg(target_os = "macos")]
        {
            &[Self::Automatic, Self::Metal]
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            &[Self::Automatic, Self::OpenGl, Self::Vulkan]
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Automatic => crate::i18n::tr!("Automatic"),
            Self::Dx12 => "DirectX 12",
            Self::OpenGl => "OpenGL",
            Self::Vulkan => "Vulkan",
            Self::Metal => "Metal",
        }
    }

    /// The wgpu backends to force, or `None` to keep the ladder.
    pub fn backends(self) -> Option<Backends> {
        match self {
            Self::Automatic => None,
            Self::Dx12 => Some(Backends::DX12),
            Self::OpenGl => Some(Backends::GL),
            Self::Vulkan => Some(Backends::VULKAN),
            Self::Metal => Some(Backends::METAL),
        }
    }
}

/// What Settings ▸ Graphics edits and what a start applies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsPrefs {
    pub display_sync: DisplaySync,
    pub frame_queue: FrameQueue,
    pub backend: BackendChoice,
}

impl GraphicsPrefs {
    pub const KEY: &'static str = "graphics.prefs";

    pub fn load(store: &Store) -> Self {
        store.setting(Self::KEY).unwrap_or_default()
    }

    pub fn save(self, store: &Store) -> Result<(), String> {
        store
            .set_setting(Self::KEY, &self)
            .map_err(|error| error.to_string())
    }

    /// The egui-wgpu surface config these prefs ask for.
    pub fn surface_config(self) -> SurfaceConfig {
        SurfaceConfig {
            present_mode: self.display_sync.present_mode(),
            desired_maximum_frame_latency: Some(self.frame_queue.latency()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_low_latency_fix() {
        let prefs = GraphicsPrefs::default();
        let surface = prefs.surface_config();
        assert_eq!(surface.present_mode, PresentMode::AutoNoVsync);
        assert_eq!(surface.desired_maximum_frame_latency, Some(1));
        assert!(!prefs.display_sync.vsync_on());
        assert_eq!(prefs.backend, BackendChoice::Automatic);
    }

    #[test]
    fn switches_map_to_display_sync() {
        assert_eq!(
            DisplaySync::from_switches(false, false),
            DisplaySync::LowLatency
        );
        assert_eq!(DisplaySync::from_switches(true, false), DisplaySync::Vsync);
        assert_eq!(
            DisplaySync::from_switches(false, true),
            DisplaySync::Uncapped
        );
        assert_eq!(
            DisplaySync::from_switches(true, true),
            DisplaySync::Vsync,
            "VSync wins over uncapped"
        );
    }

    #[test]
    fn round_trips_through_json() {
        let prefs = GraphicsPrefs {
            display_sync: DisplaySync::Uncapped,
            frame_queue: FrameQueue::Two,
            backend: BackendChoice::OpenGl,
        };
        let json = serde_json::to_string(&prefs).unwrap();
        assert_eq!(serde_json::from_str::<GraphicsPrefs>(&json).unwrap(), prefs);
    }
}
