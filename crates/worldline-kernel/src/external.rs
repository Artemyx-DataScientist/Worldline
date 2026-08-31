//! External runtime authority handles (M0.6 stable external plugin boundary).
//!
//! External plugins refer to delegated authority without receiving kernel
//! grant or principal internals. A handle is host-created, scoped to exactly
//! one [`RuntimeId`], attenuated to explicit operations and resources,
//! revocable, and never persistent. Handle values are sequence numbers in a
//! per-kernel table; security never relies on value secrecy or
//! unguessability. Every use is resolved against the exact owning runtime,
//! so a copied, guessed, or replayed value can never reach another runtime's
//! authority. A runtime terminal path (deactivate, unregister, stop, crash)
//! revokes every handle of that runtime, and a restarted runtime receives
//! fresh values that never collide with revoked ones.

use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::RuntimeId;
use crate::security::{OperationId, ResourceId};

/// A live external authority handle with its attenuation metadata.
#[derive(Clone, Debug)]
pub(crate) struct LiveExternalHandle {
    runtime: RuntimeId,
    operations: BTreeSet<OperationId>,
    resources: BTreeSet<ResourceId>,
}

/// Read-only view of a resolved handle returned to authorized callers.
#[derive(Clone, Debug)]
pub struct ExternalHandleView {
    handle: u64,
    runtime: RuntimeId,
    operations: BTreeSet<OperationId>,
    resources: BTreeSet<ResourceId>,
}

impl ExternalHandleView {
    pub const fn handle(&self) -> u64 {
        self.handle
    }

    pub fn runtime(&self) -> &RuntimeId {
        &self.runtime
    }

    pub fn operations(&self) -> &BTreeSet<OperationId> {
        &self.operations
    }

    pub fn resources(&self) -> &BTreeSet<ResourceId> {
        &self.resources
    }
}

/// Host-side handle table. Values are monotonic per kernel instance, so a
/// restarted runtime never receives a previously used value and old handles
/// can never alias new authority.
#[derive(Default)]
pub(crate) struct ExternalHandleTable {
    next_value: u64,
    live: BTreeMap<u64, LiveExternalHandle>,
    revoked: BTreeSet<u64>,
}

impl ExternalHandleTable {
    /// Issues a new attenuated handle bound to `runtime`. Empty operation or
    /// resource sets are valid and authorize nothing (default deny).
    pub fn issue(
        &mut self,
        runtime: &RuntimeId,
        operations: BTreeSet<OperationId>,
        resources: BTreeSet<ResourceId>,
    ) -> u64 {
        let handle = self.next_value;
        self.next_value += 1;
        self.live.insert(
            handle,
            LiveExternalHandle {
                runtime: *runtime,
                operations,
                resources,
            },
        );
        handle
    }

    /// Revokes every handle owned by `runtime` (terminal lifecycle path).
    /// Returns the number of revoked live handles.
    pub fn close_runtime(&mut self, runtime: &RuntimeId) -> usize {
        let doomed: Vec<u64> = self
            .live
            .iter()
            .filter(|(_, live)| live.runtime == *runtime)
            .map(|(handle, _)| *handle)
            .collect();
        for handle in &doomed {
            self.live.remove(handle);
            self.revoked.insert(*handle);
        }
        doomed.len()
    }

    /// Revokes one handle. Revocation is idempotent; only the owning runtime
    /// may revoke.
    pub fn revoke(&mut self, runtime: &RuntimeId, handle: u64) -> Result<(), crate::KernelError> {
        match self.live.get(&handle) {
            Some(live) if live.runtime == *runtime => {
                self.live.remove(&handle);
                self.revoked.insert(handle);
                Ok(())
            }
            Some(live) => Err(crate::KernelError::ExternalHandleWrongRuntime {
                handle,
                claimed: *runtime,
                owner: live.runtime,
            }),
            None if self.revoked.contains(&handle) => {
                Err(crate::KernelError::ExternalHandleRevoked { handle })
            }
            None => Err(crate::KernelError::InvalidExternalHandle { handle }),
        }
    }

    /// Resolves a handle for the claiming runtime or returns the exact typed
    /// denial. Resolution order is deterministic: live-and-owned, live
    /// elsewhere (wrong runtime), revoked, unknown.
    pub fn resolve(
        &self,
        runtime: &RuntimeId,
        handle: u64,
    ) -> Result<&LiveExternalHandle, crate::KernelError> {
        match self.live.get(&handle) {
            Some(live) if live.runtime == *runtime => Ok(live),
            Some(live) => Err(crate::KernelError::ExternalHandleWrongRuntime {
                handle,
                claimed: *runtime,
                owner: live.runtime,
            }),
            None if self.revoked.contains(&handle) => {
                Err(crate::KernelError::ExternalHandleRevoked { handle })
            }
            None => Err(crate::KernelError::InvalidExternalHandle { handle }),
        }
    }

    /// Resolves and verifies that `operation` over `resource` is inside the
    /// attenuation encoded at issue time. Attenuation can never be widened by
    /// a guest.
    pub fn check_scope(
        &self,
        runtime: &RuntimeId,
        handle: u64,
        operation: &OperationId,
        resource: &ResourceId,
    ) -> Result<(), crate::KernelError> {
        let live = self.resolve(runtime, handle)?;
        if live.operations.contains(operation) && live.resources.contains(resource) {
            Ok(())
        } else {
            Err(crate::KernelError::ExternalHandleScopeDenied {
                handle,
                runtime: *runtime,
            })
        }
    }

    /// Read-only view for host diagnostics.
    pub fn view(
        &self,
        runtime: &RuntimeId,
        handle: u64,
    ) -> Result<ExternalHandleView, crate::KernelError> {
        let live = self.resolve(runtime, handle)?;
        Ok(ExternalHandleView {
            handle,
            runtime: live.runtime,
            operations: live.operations.clone(),
            resources: live.resources.clone(),
        })
    }

    pub fn live_count(&self, runtime: &RuntimeId) -> usize {
        self.live
            .values()
            .filter(|live| live.runtime == *runtime)
            .count()
    }
}
