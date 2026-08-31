//! WASI permission classes for the WASM component boundary.
//!
//! Principle (ADR "Supported WASI surface"): a WASM component starts with no
//! ambient capability. The permission classes below are explicit and
//! host-scoped; a permission *request* in a package manifest never equals a
//! *granted* permission, and prohibitions exist before the first guest call —
//! the host never links the full WASI world and denies inside host functions.
//!
//! v1 implements only the denied path: [`WasmPluginHost`](crate::WasmPluginHost)
//! registers **no** WASI bindings at all, so a component importing any
//! `wasi:*` interface fails to link with
//! [`WasmHostError::UnsupportedExternalAbi`](crate::WasmHostError::UnsupportedExternalAbi).
//! This type is kept so a future change can add explicit scoped grants
//! (preopened directories, scoped network, ...) without reworking the
//! boundary contract.

use worldline_plugin_protocol::PermissionClass;

/// The classes of WASI surface a host policy can grant to a component.
///
/// Deliberately the same class vocabulary as the manifest's
/// [`PermissionClass`]: the manifest requests, this set records what a host
/// policy actually granted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WasiPermissionClass {
    /// Scoped filesystem access (only ever explicit preopened directories).
    Filesystem,
    /// Scoped network access (no inbound/outbound without an explicit grant).
    Network,
    /// Wall-clock and monotonic time.
    Clock,
    /// Randomness source (never granted implicitly).
    Random,
    /// Environment variables (never inherited by default).
    Environment,
}

impl From<PermissionClass> for WasiPermissionClass {
    fn from(class: PermissionClass) -> Self {
        match class {
            PermissionClass::Filesystem => Self::Filesystem,
            PermissionClass::Network => Self::Network,
            PermissionClass::Clock => Self::Clock,
            PermissionClass::Random => Self::Random,
            PermissionClass::Environment => Self::Environment,
        }
    }
}

/// The granted WASI permission set of one component.
///
/// All classes are default-denied. The only constructible value in v1 is the
/// all-denied default: the fields are private, no constructor accepts grants,
/// and the host registers no WASI bindings, so every component runs with zero
/// ambient authority and `wasi:*` imports fail to link.
///
/// # Invariant
///
/// A manifest request never equals a granted permission. Values of this type
/// are only ever produced by explicit host policy (never by parsing a
/// manifest), and in v1 the single value in existence is the all-denied
/// [`WasiPermissionSet::default()`]. When a future change introduces explicit
/// scoped grants, it must keep that separation: requests flow from manifests
/// into host policy review; grants flow out of host policy into this set.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WasiPermissionSet {
    filesystem: bool,
    network: bool,
    clock: bool,
    random: bool,
    environment: bool,
}

impl WasiPermissionSet {
    /// The all-denied set: the only set that exists in v1.
    pub const NONE: Self = Self {
        filesystem: false,
        network: false,
        clock: false,
        random: false,
        environment: false,
    };

    /// Returns whether `class` is granted by this set.
    ///
    /// Always `false` in v1, matching the fact that the host registers no
    /// WASI bindings.
    pub fn is_granted(&self, class: WasiPermissionClass) -> bool {
        match class {
            WasiPermissionClass::Filesystem => self.filesystem,
            WasiPermissionClass::Network => self.network,
            WasiPermissionClass::Clock => self.clock,
            WasiPermissionClass::Random => self.random,
            WasiPermissionClass::Environment => self.environment,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_grants_nothing() {
        let set = WasiPermissionSet::default();
        for class in [
            WasiPermissionClass::Filesystem,
            WasiPermissionClass::Network,
            WasiPermissionClass::Clock,
            WasiPermissionClass::Random,
            WasiPermissionClass::Environment,
        ] {
            assert!(!set.is_granted(class));
            assert!(!WasiPermissionSet::NONE.is_granted(class));
        }
    }

    #[test]
    fn manifest_classes_map_to_wasi_classes() {
        assert_eq!(
            WasiPermissionClass::from(PermissionClass::Filesystem),
            WasiPermissionClass::Filesystem
        );
        assert_eq!(
            WasiPermissionClass::from(PermissionClass::Environment),
            WasiPermissionClass::Environment
        );
    }
}
