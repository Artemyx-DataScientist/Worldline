use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use crate::{
    PluginId,
    trajectory::{Trajectory, TrajectoryEventKind},
};

/// Opaque stable identity of one installed plugin definition.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstallationId(String);

impl InstallationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn generated(sequence: u64) -> Self {
        Self(format!("installation-{sequence}"))
    }
}

impl From<&InstallationId> for InstallationId {
    fn from(value: &InstallationId) -> Self {
        value.clone()
    }
}

impl fmt::Display for InstallationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Explicit version of the bytes stored by an installation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StateSchemaVersion(u64);

impl StateSchemaVersion {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for StateSchemaVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Opaque key inside one installation's state namespace.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StateKey(String);

impl StateKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_well_formed(&self) -> bool {
        !self.0.trim().is_empty() && !self.0.contains('\0')
    }
}

impl From<&str> for StateKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for StateKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&StateKey> for StateKey {
    fn from(value: &StateKey) -> Self {
        value.clone()
    }
}

impl fmt::Display for StateKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque bytes stored by the state API.
pub type StateValue = Vec<u8>;

/// Opaque identity of one state transaction.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StateTransactionId(String);

impl StateTransactionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StateTransactionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Stable identity of one directed schema migration.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MigrationId(String);

impl MigrationId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for MigrationId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for MigrationId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for MigrationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Lifecycle of the persistent installation state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InstallationStatus {
    Installed,
    PreparingState,
    Migrating,
    Ready,
    MigrationFailed,
    Uninstalling,
}

impl fmt::Display for InstallationStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Installed => "Installed",
            Self::PreparingState => "PreparingState",
            Self::Migrating => "Migrating",
            Self::Ready => "Ready",
            Self::MigrationFailed => "MigrationFailed",
            Self::Uninstalling => "Uninstalling",
        };
        formatter.write_str(name)
    }
}

/// Kernel-owned metadata for one persistent plugin installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallationRecord {
    installation_id: InstallationId,
    plugin_id: PluginId,
    state_schema_version: StateSchemaVersion,
    status: InstallationStatus,
}

impl InstallationRecord {
    pub fn installation_id(&self) -> &InstallationId {
        &self.installation_id
    }

    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub const fn state_schema_version(&self) -> StateSchemaVersion {
        self.state_schema_version
    }

    pub const fn status(&self) -> InstallationStatus {
        self.status
    }

    pub(crate) fn new(
        installation_id: InstallationId,
        plugin_id: PluginId,
        state_schema_version: StateSchemaVersion,
        status: InstallationStatus,
    ) -> Self {
        Self {
            installation_id,
            plugin_id,
            state_schema_version,
            status,
        }
    }

    fn with_status(&self, status: InstallationStatus) -> Self {
        Self::new(
            self.installation_id.clone(),
            self.plugin_id.clone(),
            self.state_schema_version,
            status,
        )
    }

    fn with_schema_and_status(
        &self,
        state_schema_version: StateSchemaVersion,
        status: InstallationStatus,
    ) -> Self {
        Self::new(
            self.installation_id.clone(),
            self.plugin_id.clone(),
            state_schema_version,
            status,
        )
    }
}

/// Errors at the installation/state boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateError {
    UnknownInstallation {
        installation: InstallationId,
    },
    InstallationAlreadyExists {
        installation: InstallationId,
    },
    InstallationNotReady {
        installation: InstallationId,
        status: InstallationStatus,
    },
    StateAccessDenied {
        installation: InstallationId,
    },
    StateSchemaIncompatible {
        persisted: StateSchemaVersion,
        target: StateSchemaVersion,
    },
    NoMigrationPath {
        from: StateSchemaVersion,
        to: StateSchemaVersion,
    },
    AmbiguousMigrationPath {
        from: StateSchemaVersion,
        to: StateSchemaVersion,
    },
    MigrationFailed {
        installation: InstallationId,
        migration: Option<MigrationId>,
        message: String,
    },
    TransactionCommitFailed {
        installation: InstallationId,
        transaction: StateTransactionId,
    },
    UninstallFailed {
        installation: InstallationId,
    },
    RuntimeInstallationMismatch {
        expected: InstallationId,
        actual: InstallationId,
    },
    InvalidStateKey {
        key: StateKey,
    },
    InvalidMigration {
        migration: MigrationId,
        reason: String,
    },
    BackendFailure {
        operation: &'static str,
        message: String,
    },
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownInstallation { installation } => {
                write!(formatter, "installation '{installation}' is unknown")
            }
            Self::InstallationAlreadyExists { installation } => {
                write!(formatter, "installation '{installation}' already exists")
            }
            Self::InstallationNotReady {
                installation,
                status,
            } => write!(
                formatter,
                "installation '{installation}' is not ready (status: {status})"
            ),
            Self::StateAccessDenied { installation } => write!(
                formatter,
                "state access is denied for installation '{installation}'"
            ),
            Self::StateSchemaIncompatible { persisted, target } => write!(
                formatter,
                "state schema '{persisted}' is incompatible with target '{target}'"
            ),
            Self::NoMigrationPath { from, to } => {
                write!(
                    formatter,
                    "no migration path exists from schema '{from}' to '{to}'"
                )
            }
            Self::AmbiguousMigrationPath { from, to } => write!(
                formatter,
                "migration path from schema '{from}' to '{to}' is ambiguous"
            ),
            Self::MigrationFailed {
                installation,
                migration,
                message,
            } => write!(
                formatter,
                "migration for installation '{installation}' failed at {migration:?}: {message}"
            ),
            Self::TransactionCommitFailed {
                installation,
                transaction,
            } => write!(
                formatter,
                "transaction '{transaction}' for installation '{installation}' did not commit"
            ),
            Self::UninstallFailed { installation } => {
                write!(
                    formatter,
                    "uninstall of installation '{installation}' failed"
                )
            }
            Self::RuntimeInstallationMismatch { expected, actual } => write!(
                formatter,
                "runtime is bound to installation '{expected}', not '{actual}'"
            ),
            Self::InvalidStateKey { key } => write!(formatter, "invalid state key '{key}'"),
            Self::InvalidMigration { migration, reason } => {
                write!(formatter, "invalid migration '{migration}': {reason}")
            }
            Self::BackendFailure { operation, message } => {
                write!(formatter, "state backend {operation} failed: {message}")
            }
        }
    }
}

impl std::error::Error for StateError {}

/// Error returned by a migration step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationError {
    message: String,
}

impl MigrationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<&str> for MigrationError {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for MigrationError {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<StateError> for MigrationError {
    fn from(value: StateError) -> Self {
        Self::new(value.to_string())
    }
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MigrationError {}

/// A backend snapshot exchanged only between the kernel and a StateBackend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendState {
    record: InstallationRecord,
    values: BTreeMap<StateKey, StateValue>,
}

impl BackendState {
    pub fn new(record: InstallationRecord, values: BTreeMap<StateKey, StateValue>) -> Self {
        Self { record, values }
    }

    pub fn record(&self) -> &InstallationRecord {
        &self.record
    }

    pub fn values(&self) -> &BTreeMap<StateKey, StateValue> {
        &self.values
    }
}

/// Abstract persistence contract owned by the kernel.
pub trait StateBackend: Send + Sync {
    fn create(&self, state: BackendState) -> Result<(), StateError>;

    fn snapshot(&self, installation: &InstallationId) -> Result<BackendState, StateError>;

    fn commit(&self, installation: &InstallationId, state: BackendState) -> Result<(), StateError>;

    fn update_record(&self, record: InstallationRecord) -> Result<(), StateError>;

    fn delete(&self, installation: &InstallationId) -> Result<(), StateError>;

    fn list_records(&self) -> Vec<InstallationRecord>;
}

/// Deterministic in-memory StateBackend used by the bootstrap kernel and tests.
#[derive(Default)]
pub struct InMemoryStateBackend {
    installations: RwLock<BTreeMap<InstallationId, BackendState>>,
    fail_next_commit: AtomicBool,
    fail_next_delete: AtomicBool,
}

impl InMemoryStateBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Injects one atomic commit failure. The next commit consumes the fault.
    pub fn fail_next_commit(&self) {
        self.fail_next_commit.store(true, Ordering::SeqCst);
    }

    /// Injects one uninstall/delete failure. The next delete consumes the fault.
    pub fn fail_next_delete(&self) {
        self.fail_next_delete.store(true, Ordering::SeqCst);
    }
}

impl StateBackend for InMemoryStateBackend {
    fn create(&self, state: BackendState) -> Result<(), StateError> {
        let installation = state.record.installation_id.clone();
        let mut installations = self
            .installations
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if installations.contains_key(&installation) {
            return Err(StateError::InstallationAlreadyExists { installation });
        }
        installations.insert(installation, state);
        Ok(())
    }

    fn snapshot(&self, installation: &InstallationId) -> Result<BackendState, StateError> {
        self.installations
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(installation)
            .cloned()
            .ok_or_else(|| StateError::UnknownInstallation {
                installation: installation.clone(),
            })
    }

    fn commit(&self, installation: &InstallationId, state: BackendState) -> Result<(), StateError> {
        if self.fail_next_commit.swap(false, Ordering::SeqCst) {
            return Err(StateError::BackendFailure {
                operation: "commit",
                message: "injected commit failure".to_owned(),
            });
        }
        let mut installations = self
            .installations
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !installations.contains_key(installation) {
            return Err(StateError::UnknownInstallation {
                installation: installation.clone(),
            });
        }
        installations.insert(installation.clone(), state);
        Ok(())
    }

    fn update_record(&self, record: InstallationRecord) -> Result<(), StateError> {
        let installation = record.installation_id.clone();
        let mut installations = self
            .installations
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = installations.get_mut(&installation) else {
            return Err(StateError::UnknownInstallation { installation });
        };
        state.record = record;
        Ok(())
    }

    fn delete(&self, installation: &InstallationId) -> Result<(), StateError> {
        if self.fail_next_delete.swap(false, Ordering::SeqCst) {
            return Err(StateError::BackendFailure {
                operation: "delete",
                message: "injected delete failure".to_owned(),
            });
        }
        let mut installations = self
            .installations
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if installations.remove(installation).is_none() {
            return Err(StateError::UnknownInstallation {
                installation: installation.clone(),
            });
        }
        Ok(())
    }

    fn list_records(&self) -> Vec<InstallationRecord> {
        self.installations
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .map(|state| state.record.clone())
            .collect()
    }
}

/// Directed migration with a kernel-controlled state-only execution context.
type MigrationFunction = dyn Fn(&mut MigrationContext) -> Result<(), MigrationError> + Send + Sync;

pub struct StateMigration {
    migration_id: MigrationId,
    from_schema: StateSchemaVersion,
    to_schema: StateSchemaVersion,
    function: Arc<MigrationFunction>,
}

impl Clone for StateMigration {
    fn clone(&self) -> Self {
        Self {
            migration_id: self.migration_id.clone(),
            from_schema: self.from_schema,
            to_schema: self.to_schema,
            function: Arc::clone(&self.function),
        }
    }
}

impl fmt::Debug for StateMigration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateMigration")
            .field("migration_id", &self.migration_id)
            .field("from_schema", &self.from_schema)
            .field("to_schema", &self.to_schema)
            .finish_non_exhaustive()
    }
}

impl PartialEq for StateMigration {
    fn eq(&self, other: &Self) -> bool {
        self.migration_id == other.migration_id
            && self.from_schema == other.from_schema
            && self.to_schema == other.to_schema
    }
}

impl Eq for StateMigration {}

impl StateMigration {
    pub fn new<F>(
        migration_id: impl Into<MigrationId>,
        from_schema: StateSchemaVersion,
        to_schema: StateSchemaVersion,
        function: F,
    ) -> Self
    where
        F: Fn(&mut MigrationContext) -> Result<(), MigrationError> + Send + Sync + 'static,
    {
        Self {
            migration_id: migration_id.into(),
            from_schema,
            to_schema,
            function: Arc::new(function),
        }
    }

    pub fn migration_id(&self) -> &MigrationId {
        &self.migration_id
    }

    pub const fn from_schema(&self) -> StateSchemaVersion {
        self.from_schema
    }

    pub const fn to_schema(&self) -> StateSchemaVersion {
        self.to_schema
    }

    fn validate(&self) -> Result<(), StateError> {
        if self.migration_id.as_str().trim().is_empty() {
            return Err(StateError::InvalidMigration {
                migration: self.migration_id.clone(),
                reason: "migration id must not be empty".to_owned(),
            });
        }
        if self.from_schema == self.to_schema {
            return Err(StateError::InvalidMigration {
                migration: self.migration_id.clone(),
                reason: "migration source and target schemas must differ".to_owned(),
            });
        }
        Ok(())
    }

    fn execute(&self, context: &mut MigrationContext) -> Result<(), MigrationError> {
        (self.function)(context)
    }
}

/// Deterministic directed migration plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationPlan {
    from_schema: StateSchemaVersion,
    to_schema: StateSchemaVersion,
    steps: Vec<StateMigration>,
}

impl MigrationPlan {
    pub fn from_schema(&self) -> StateSchemaVersion {
        self.from_schema
    }

    pub fn to_schema(&self) -> StateSchemaVersion {
        self.to_schema
    }

    pub fn steps(&self) -> &[StateMigration] {
        &self.steps
    }

    pub fn migration_ids(&self) -> Vec<MigrationId> {
        self.steps
            .iter()
            .map(|migration| migration.migration_id.clone())
            .collect()
    }

    pub(crate) fn build(
        from_schema: StateSchemaVersion,
        to_schema: StateSchemaVersion,
        migrations: &[StateMigration],
    ) -> Result<Self, StateError> {
        if from_schema == to_schema {
            return Ok(Self {
                from_schema,
                to_schema,
                steps: Vec::new(),
            });
        }

        let mut edges = migrations.to_vec();
        for migration in &edges {
            migration.validate()?;
        }
        edges.sort_by(|left, right| {
            (left.from_schema, left.to_schema, &left.migration_id).cmp(&(
                right.from_schema,
                right.to_schema,
                &right.migration_id,
            ))
        });
        for pair in edges.windows(2) {
            if pair[0].from_schema == pair[1].from_schema && pair[0].to_schema == pair[1].to_schema
            {
                return Err(StateError::InvalidMigration {
                    migration: pair[1].migration_id.clone(),
                    reason: "duplicate migration edge".to_owned(),
                });
            }
        }
        let mut ids = BTreeSet::new();
        for migration in &edges {
            if !ids.insert(migration.migration_id.clone()) {
                return Err(StateError::InvalidMigration {
                    migration: migration.migration_id.clone(),
                    reason: "duplicate migration id".to_owned(),
                });
            }
        }

        let mut paths = Vec::new();
        let mut visited = BTreeSet::from([from_schema]);
        let mut path = Vec::new();
        find_paths(
            from_schema,
            to_schema,
            &edges,
            &mut visited,
            &mut path,
            &mut paths,
        );
        match paths.len() {
            0 => Err(StateError::NoMigrationPath {
                from: from_schema,
                to: to_schema,
            }),
            1 => Ok(Self {
                from_schema,
                to_schema,
                steps: paths.pop().expect("one migration path must exist"),
            }),
            _ => Err(StateError::AmbiguousMigrationPath {
                from: from_schema,
                to: to_schema,
            }),
        }
    }
}

fn find_paths(
    current: StateSchemaVersion,
    target: StateSchemaVersion,
    edges: &[StateMigration],
    visited: &mut BTreeSet<StateSchemaVersion>,
    path: &mut Vec<StateMigration>,
    paths: &mut Vec<Vec<StateMigration>>,
) {
    if paths.len() > 1 {
        return;
    }
    if current == target {
        paths.push(path.clone());
        return;
    }
    for edge in edges.iter().filter(|edge| edge.from_schema == current) {
        if !visited.insert(edge.to_schema) {
            continue;
        }
        path.push(edge.clone());
        find_paths(edge.to_schema, target, edges, visited, path, paths);
        path.pop();
        visited.remove(&edge.to_schema);
        if paths.len() > 1 {
            return;
        }
    }
}

/// State-only context passed to migration code.
pub struct MigrationContext {
    transaction: StateTransaction,
    installation: InstallationId,
    from_schema: StateSchemaVersion,
    to_schema: StateSchemaVersion,
}

impl MigrationContext {
    pub fn installation_id(&self) -> &InstallationId {
        &self.installation
    }

    pub const fn from_schema(&self) -> StateSchemaVersion {
        self.from_schema
    }

    pub const fn to_schema(&self) -> StateSchemaVersion {
        self.to_schema
    }

    pub fn get(&self, key: impl Into<StateKey>) -> Option<StateValue> {
        self.transaction.get(key)
    }

    pub fn contains(&self, key: impl Into<StateKey>) -> bool {
        self.transaction.contains(key)
    }

    pub fn list_keys(&self) -> Vec<StateKey> {
        self.transaction.list_keys()
    }

    pub fn put(
        &mut self,
        key: impl Into<StateKey>,
        value: impl AsRef<[u8]>,
    ) -> Result<(), StateError> {
        self.transaction.put(key, value)
    }

    pub fn delete(&mut self, key: impl Into<StateKey>) -> Result<bool, StateError> {
        self.transaction.delete(key)
    }

    fn set_step_schemas(&mut self, from_schema: StateSchemaVersion, to_schema: StateSchemaVersion) {
        self.from_schema = from_schema;
        self.to_schema = to_schema;
    }

    fn into_transaction(self) -> StateTransaction {
        self.transaction
    }
}

/// Kernel-bound read and transaction handle for exactly one installation.
#[derive(Clone)]
pub struct StateHandle {
    store: Arc<StateStore>,
    installation: InstallationId,
}

impl fmt::Debug for StateHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateHandle")
            .field("installation", &self.installation)
            .finish_non_exhaustive()
    }
}

impl StateHandle {
    pub fn installation_id(&self) -> &InstallationId {
        &self.installation
    }

    pub fn get(&self, key: impl Into<StateKey>) -> Result<Option<StateValue>, StateError> {
        self.store.read_value(&self.installation, key.into())
    }

    pub fn contains(&self, key: impl Into<StateKey>) -> Result<bool, StateError> {
        Ok(self.get(key)?.is_some())
    }

    pub fn list_keys(&self) -> Result<Vec<StateKey>, StateError> {
        self.store.list_keys(&self.installation)
    }

    pub fn transaction(&self) -> Result<StateTransaction, StateError> {
        self.store.begin_transaction(&self.installation)
    }
}

/// Transactional view of one installation's state namespace.
pub struct StateTransaction {
    store: Arc<StateStore>,
    installation: InstallationId,
    transaction: StateTransactionId,
    schema: StateSchemaVersion,
    original_values: BTreeMap<StateKey, StateValue>,
    values: BTreeMap<StateKey, StateValue>,
    active: bool,
}

impl fmt::Debug for StateTransaction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateTransaction")
            .field("installation", &self.installation)
            .field("transaction", &self.transaction)
            .field("schema", &self.schema)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl StateTransaction {
    pub fn id(&self) -> &StateTransactionId {
        &self.transaction
    }

    pub fn installation_id(&self) -> &InstallationId {
        &self.installation
    }

    pub const fn schema_version(&self) -> StateSchemaVersion {
        self.schema
    }

    pub fn get(&self, key: impl Into<StateKey>) -> Option<StateValue> {
        self.values.get(&key.into()).cloned()
    }

    pub fn contains(&self, key: impl Into<StateKey>) -> bool {
        self.get(key).is_some()
    }

    pub fn list_keys(&self) -> Vec<StateKey> {
        self.values.keys().cloned().collect()
    }

    pub fn put(
        &mut self,
        key: impl Into<StateKey>,
        value: impl AsRef<[u8]>,
    ) -> Result<(), StateError> {
        let key = key.into();
        if !key.is_well_formed() {
            return Err(StateError::InvalidStateKey { key });
        }
        self.values.insert(key, value.as_ref().to_vec());
        Ok(())
    }

    pub fn delete(&mut self, key: impl Into<StateKey>) -> Result<bool, StateError> {
        let key = key.into();
        if !key.is_well_formed() {
            return Err(StateError::InvalidStateKey { key });
        }
        Ok(self.values.remove(&key).is_some())
    }

    pub fn commit(mut self) -> Result<(), StateError> {
        let values = std::mem::take(&mut self.values);
        let changed_key_count = changed_key_count(&self.original_values, &values);
        self.active = false;
        self.store.commit_transaction(
            &self.installation,
            &self.transaction,
            self.schema,
            values,
            changed_key_count,
        )
    }

    pub fn rollback(mut self) -> Result<(), StateError> {
        if self.active {
            self.active = false;
            self.store
                .log_transaction_rollback(&self.installation, &self.transaction);
        }
        Ok(())
    }

    fn commit_with_schema(
        mut self,
        schema: StateSchemaVersion,
        status: InstallationStatus,
    ) -> Result<(), StateError> {
        let values = std::mem::take(&mut self.values);
        let changed_key_count = changed_key_count(&self.original_values, &values);
        self.active = false;
        self.store.commit_transaction_with_schema(
            &self.installation,
            &self.transaction,
            schema,
            status,
            values,
            changed_key_count,
        )
    }
}

impl Drop for StateTransaction {
    fn drop(&mut self) {
        if self.active {
            self.active = false;
            self.store
                .log_transaction_rollback(&self.installation, &self.transaction);
        }
    }
}

fn changed_key_count(
    original: &BTreeMap<StateKey, StateValue>,
    current: &BTreeMap<StateKey, StateValue>,
) -> usize {
    original
        .keys()
        .chain(current.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| original.get(*key) != current.get(*key))
        .count()
}

pub(crate) struct StateStore {
    backend: Arc<dyn StateBackend>,
    records: RwLock<BTreeMap<InstallationId, InstallationRecord>>,
    next_installation: AtomicU64,
    next_transaction: AtomicU64,
    trajectory: Trajectory,
}

impl StateStore {
    pub(crate) fn new(backend: Arc<dyn StateBackend>, trajectory: Trajectory) -> Self {
        let records = backend
            .list_records()
            .into_iter()
            .map(|record| (record.installation_id.clone(), record))
            .collect::<BTreeMap<_, _>>();
        let next_installation = records
            .keys()
            .filter_map(|id| id.as_str().strip_prefix("installation-"))
            .filter_map(|suffix| suffix.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        Self {
            backend,
            records: RwLock::new(records),
            next_installation: AtomicU64::new(next_installation),
            next_transaction: AtomicU64::new(0),
            trajectory,
        }
    }

    pub(crate) fn create_installation(
        &self,
        plugin_id: PluginId,
        schema: StateSchemaVersion,
    ) -> Result<InstallationId, StateError> {
        let installation = loop {
            let sequence = self.next_installation.fetch_add(1, Ordering::SeqCst) + 1;
            let candidate = InstallationId::generated(sequence);
            let exists = self
                .records
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains_key(&candidate);
            if !exists {
                break candidate;
            }
        };
        let installed = InstallationRecord::new(
            installation.clone(),
            plugin_id.clone(),
            schema,
            InstallationStatus::Installed,
        );
        self.backend
            .create(BackendState::new(installed.clone(), BTreeMap::new()))?;
        let ready = installed.with_status(InstallationStatus::Ready);
        if let Err(error) = self.backend.update_record(ready.clone()) {
            let _ = self.backend.delete(&installation);
            return Err(error);
        }
        self.records
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(installation.clone(), ready);
        self.trajectory
            .push_security(TrajectoryEventKind::InstallationCreated {
                installation: installation.clone(),
                plugin: plugin_id,
                schema,
            });
        self.trajectory
            .push_security(TrajectoryEventKind::InstallationReady {
                installation: installation.clone(),
                schema,
            });
        Ok(installation)
    }

    pub(crate) fn record(&self, installation: &InstallationId) -> Option<InstallationRecord> {
        self.records
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(installation)
            .cloned()
    }

    pub(crate) fn records_for_plugin(&self, plugin: &PluginId) -> Vec<InstallationRecord> {
        self.records
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .filter(|record| record.plugin_id == *plugin)
            .cloned()
            .collect()
    }

    pub(crate) fn all_records(&self) -> Vec<InstallationRecord> {
        self.records
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// A new runtime registration is an explicit lifecycle/update retry after
    /// a failed migration. Automatic reconcile never calls this transition.
    pub(crate) fn prepare_retry(&self, installation: &InstallationId) -> Result<(), StateError> {
        let record = self
            .record(installation)
            .ok_or_else(|| StateError::UnknownInstallation {
                installation: installation.clone(),
            })?;
        if record.status == InstallationStatus::MigrationFailed {
            let ready = record.with_status(InstallationStatus::Ready);
            self.replace_record(&ready)?;
            self.trajectory
                .push_security(TrajectoryEventKind::InstallationReady {
                    installation: installation.clone(),
                    schema: ready.state_schema_version,
                });
        }
        Ok(())
    }

    pub(crate) fn handle(
        self: &Arc<Self>,
        installation: &InstallationId,
    ) -> Result<StateHandle, StateError> {
        self.require_state_accessible(installation)?;
        Ok(StateHandle {
            store: Arc::clone(self),
            installation: installation.clone(),
        })
    }

    pub(crate) fn begin_transaction(
        self: &Arc<Self>,
        installation: &InstallationId,
    ) -> Result<StateTransaction, StateError> {
        self.begin_transaction_for_status(installation, InstallationStatus::Ready)
    }

    fn begin_transaction_for_status(
        self: &Arc<Self>,
        installation: &InstallationId,
        required_status: InstallationStatus,
    ) -> Result<StateTransaction, StateError> {
        let record = self
            .record(installation)
            .ok_or_else(|| StateError::UnknownInstallation {
                installation: installation.clone(),
            })?;
        if record.status != required_status {
            return Err(StateError::InstallationNotReady {
                installation: installation.clone(),
                status: record.status,
            });
        }
        let snapshot = self.backend.snapshot(installation)?;
        let transaction_number = self.next_transaction.fetch_add(1, Ordering::SeqCst) + 1;
        let transaction = StateTransactionId(format!("state-transaction-{transaction_number}"));
        self.trajectory
            .push_security(TrajectoryEventKind::StateTransactionStarted {
                installation: installation.clone(),
                transaction: transaction.clone(),
            });
        Ok(StateTransaction {
            store: Arc::clone(self),
            installation: installation.clone(),
            transaction,
            schema: snapshot.record.state_schema_version,
            original_values: snapshot.values.clone(),
            values: snapshot.values,
            active: true,
        })
    }

    pub(crate) fn read_value(
        &self,
        installation: &InstallationId,
        key: StateKey,
    ) -> Result<Option<StateValue>, StateError> {
        if !key.is_well_formed() {
            return Err(StateError::InvalidStateKey { key });
        }
        self.require_state_accessible(installation)?;
        Ok(self
            .backend
            .snapshot(installation)?
            .values
            .get(&key)
            .cloned())
    }

    pub(crate) fn list_keys(
        &self,
        installation: &InstallationId,
    ) -> Result<Vec<StateKey>, StateError> {
        self.require_state_accessible(installation)?;
        Ok(self
            .backend
            .snapshot(installation)?
            .values
            .keys()
            .cloned()
            .collect())
    }

    fn require_state_accessible(&self, installation: &InstallationId) -> Result<(), StateError> {
        let Some(record) = self.record(installation) else {
            return Err(StateError::UnknownInstallation {
                installation: installation.clone(),
            });
        };
        if record.status == InstallationStatus::Uninstalling {
            return Err(StateError::InstallationNotReady {
                installation: installation.clone(),
                status: record.status,
            });
        }
        Ok(())
    }

    pub(crate) fn prepare_for_schema(
        self: &Arc<Self>,
        installation: &InstallationId,
        target_schema: StateSchemaVersion,
        migrations: &[StateMigration],
    ) -> Result<(), StateError> {
        let record = self
            .record(installation)
            .ok_or_else(|| StateError::UnknownInstallation {
                installation: installation.clone(),
            })?;
        if record.status != InstallationStatus::Ready {
            return Err(StateError::InstallationNotReady {
                installation: installation.clone(),
                status: record.status,
            });
        }
        if record.state_schema_version == target_schema {
            return Ok(());
        }

        self.replace_record(&record.with_status(InstallationStatus::PreparingState))?;
        let plan =
            match MigrationPlan::build(record.state_schema_version, target_schema, migrations) {
                Ok(plan) => plan,
                Err(error) => {
                    self.mark_migration_failed(&record, target_schema, None);
                    return Err(error);
                }
            };
        self.replace_record(&record.with_status(InstallationStatus::Migrating))?;
        self.trajectory
            .push_security(TrajectoryEventKind::MigrationPlanned {
                installation: installation.clone(),
                from_schema: plan.from_schema,
                to_schema: plan.to_schema,
                migrations: plan.migration_ids(),
            });
        self.trajectory
            .push_security(TrajectoryEventKind::MigrationStarted {
                installation: installation.clone(),
                from_schema: plan.from_schema,
                to_schema: plan.to_schema,
            });

        let transaction =
            match self.begin_transaction_for_status(installation, InstallationStatus::Migrating) {
                Ok(transaction) => transaction,
                Err(error) => {
                    self.mark_migration_failed(&record, target_schema, None);
                    return Err(error);
                }
            };
        let mut context = MigrationContext {
            transaction,
            installation: installation.clone(),
            from_schema: plan.from_schema,
            to_schema: plan.to_schema,
        };
        let mut failure: Option<(MigrationId, String)> = None;
        for migration in &plan.steps {
            context.set_step_schemas(migration.from_schema, migration.to_schema);
            self.trajectory
                .push_security(TrajectoryEventKind::MigrationStepStarted {
                    installation: installation.clone(),
                    migration: migration.migration_id.clone(),
                    from_schema: migration.from_schema,
                    to_schema: migration.to_schema,
                });
            let result = catch_unwind(AssertUnwindSafe(|| migration.execute(&mut context)));
            match result {
                Ok(Ok(())) => {
                    self.trajectory
                        .push_security(TrajectoryEventKind::MigrationStepCompleted {
                            installation: installation.clone(),
                            migration: migration.migration_id.clone(),
                            from_schema: migration.from_schema,
                            to_schema: migration.to_schema,
                        })
                }
                Ok(Err(error)) => {
                    failure = Some((migration.migration_id.clone(), error.to_string()));
                    break;
                }
                Err(_) => {
                    failure = Some((
                        migration.migration_id.clone(),
                        "migration step panicked".to_owned(),
                    ));
                    break;
                }
            }
        }
        let transaction = context.into_transaction();
        if let Some((migration, message)) = failure {
            let _ = transaction.rollback();
            let error = StateError::MigrationFailed {
                installation: installation.clone(),
                migration: Some(migration.clone()),
                message,
            };
            self.mark_migration_failed(&record, target_schema, Some(migration));
            return Err(error);
        }

        if let Err(error) = transaction.commit_with_schema(target_schema, InstallationStatus::Ready)
        {
            self.mark_migration_failed(&record, target_schema, None);
            return Err(error);
        }
        self.trajectory
            .push_security(TrajectoryEventKind::MigrationCommitted {
                installation: installation.clone(),
                from_schema: plan.from_schema,
                to_schema: plan.to_schema,
            });
        self.trajectory
            .push_security(TrajectoryEventKind::InstallationReady {
                installation: installation.clone(),
                schema: target_schema,
            });
        Ok(())
    }

    fn mark_migration_failed(
        &self,
        record: &InstallationRecord,
        target_schema: StateSchemaVersion,
        migration: Option<MigrationId>,
    ) {
        let failed = record.with_status(InstallationStatus::MigrationFailed);
        let _ = self.replace_record(&failed);
        self.trajectory
            .push_security(TrajectoryEventKind::MigrationFailed {
                installation: record.installation_id.clone(),
                from_schema: record.state_schema_version,
                to_schema: target_schema,
                migration,
            });
    }

    fn replace_record(&self, record: &InstallationRecord) -> Result<(), StateError> {
        self.backend.update_record(record.clone())?;
        self.records
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(record.installation_id.clone(), record.clone());
        Ok(())
    }

    fn commit_transaction(
        &self,
        installation: &InstallationId,
        transaction: &StateTransactionId,
        schema: StateSchemaVersion,
        values: BTreeMap<StateKey, StateValue>,
        changed_key_count: usize,
    ) -> Result<(), StateError> {
        self.commit_transaction_with_schema(
            installation,
            transaction,
            schema,
            InstallationStatus::Ready,
            values,
            changed_key_count,
        )
    }

    fn commit_transaction_with_schema(
        &self,
        installation: &InstallationId,
        transaction: &StateTransactionId,
        schema: StateSchemaVersion,
        status: InstallationStatus,
        values: BTreeMap<StateKey, StateValue>,
        changed_key_count: usize,
    ) -> Result<(), StateError> {
        let Some(record) = self.record(installation) else {
            self.log_transaction_rollback(installation, transaction);
            return Err(StateError::UnknownInstallation {
                installation: installation.clone(),
            });
        };
        let valid_status = match status {
            InstallationStatus::Ready => matches!(
                record.status,
                InstallationStatus::Ready | InstallationStatus::Migrating
            ),
            InstallationStatus::Migrating => record.status == InstallationStatus::Migrating,
            _ => false,
        };
        if !valid_status {
            self.log_transaction_rollback(installation, transaction);
            return Err(StateError::InstallationNotReady {
                installation: installation.clone(),
                status: record.status,
            });
        }
        let next_record = record.with_schema_and_status(schema, status);
        let backend_state = BackendState::new(next_record.clone(), values);
        if self.backend.commit(installation, backend_state).is_err() {
            self.log_transaction_rollback(installation, transaction);
            return Err(StateError::TransactionCommitFailed {
                installation: installation.clone(),
                transaction: transaction.clone(),
            });
        }
        self.records
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(installation.clone(), next_record.clone());
        self.trajectory
            .push_security(TrajectoryEventKind::StateTransactionCommitted {
                installation: installation.clone(),
                transaction: transaction.clone(),
                changed_key_count,
                schema,
            });
        Ok(())
    }

    fn log_transaction_rollback(
        &self,
        installation: &InstallationId,
        transaction: &StateTransactionId,
    ) {
        self.trajectory
            .push_security(TrajectoryEventKind::StateTransactionRolledBack {
                installation: installation.clone(),
                transaction: transaction.clone(),
            });
    }

    pub(crate) fn uninstall(&self, installation: &InstallationId) -> Result<(), StateError> {
        let record = self
            .record(installation)
            .ok_or_else(|| StateError::UnknownInstallation {
                installation: installation.clone(),
            })?;
        if record.status == InstallationStatus::Uninstalling {
            return Err(StateError::InstallationNotReady {
                installation: installation.clone(),
                status: record.status,
            });
        }
        let uninstalling = record.with_status(InstallationStatus::Uninstalling);
        self.replace_record(&uninstalling)?;
        self.trajectory
            .push_security(TrajectoryEventKind::InstallationUninstallStarted {
                installation: installation.clone(),
            });
        if self.backend.delete(installation).is_err() {
            let _ = self.replace_record(&record);
            self.trajectory
                .push_security(TrajectoryEventKind::InstallationUninstallFailed {
                    installation: installation.clone(),
                });
            return Err(StateError::UninstallFailed {
                installation: installation.clone(),
            });
        }
        self.records
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(installation);
        self.trajectory
            .push_security(TrajectoryEventKind::InstallationUninstalled {
                installation: installation.clone(),
            });
        Ok(())
    }
}
