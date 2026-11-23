//! The graphics backends a start may use, in the order they are tried. Data only: `record.rs` decides
//! which one a start takes, from what earlier starts left behind.

use eframe::wgpu::Backends;

/// One way to start the renderer: the wgpu backends the instance may probe.
pub struct Rung {
    pub name: &'static str,
    pub backends: Backends,
}

/// Windows: DirectX 12 is on every Windows 10 and later, and the Vulkan driver is the usual place a
/// start dies without a trace (drivers and injected layers that fault while the instance probes it),
/// so it is the last resort, not part of the default.
#[cfg(windows)]
pub const LADDER: &[Rung] = &[
    Rung {
        name: "dx12",
        backends: Backends::DX12,
    },
    Rung {
        name: "gl",
        backends: Backends::GL,
    },
    Rung {
        name: "vulkan",
        backends: Backends::VULKAN,
    },
];

/// macOS has one renderer, Metal.
#[cfg(target_os = "macos")]
pub const LADDER: &[Rung] = &[Rung {
    name: "metal",
    backends: Backends::METAL,
}];

/// Elsewhere: Vulkan with GL as its companion, then each alone.
#[cfg(not(any(windows, target_os = "macos")))]
pub const LADDER: &[Rung] = &[
    Rung {
        name: "vulkan+gl",
        backends: Backends::VULKAN.union(Backends::GL),
    },
    Rung {
        name: "gl",
        backends: Backends::GL,
    },
    Rung {
        name: "vulkan",
        backends: Backends::VULKAN,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_has_a_rung_and_unique_names() {
        assert!(!LADDER.is_empty());
        for (at, rung) in LADDER.iter().enumerate() {
            assert!(!rung.backends.is_empty(), "{} probes nothing", rung.name);
            assert!(LADDER[..at].iter().all(|other| other.name != rung.name));
        }
    }
}
