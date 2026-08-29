use std::{collections::BTreeMap, fmt};

use crate::{CausationRef, CorrelationId, EventEnvelope, InstallationId, PrincipalId};

/// Storage format is independent from product, plugin, capability and state
/// schema versions.
pub const CURRENT_STORAGE_FORMAT_VERSION: StorageFormatVersion = StorageFormatVersion::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorageFormatVersion(u32);

impl StorageFormatVersion {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for StorageFormatVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Explicit failures shared by host-side persistence adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistenceError {
    StorageOpenFailed {
        message: String,
    },
    UnsupportedStorageFormat {
        found: StorageFormatVersion,
        supported: StorageFormatVersion,
    },
    StorageCorrupt {
        message: String,
    },
    StorageIoFailure {
        operation: String,
        message: String,
    },
    StorageBusy {
        message: String,
    },
    DurabilityFailure {
        message: String,
    },
    PersistentCasConflict {
        message: String,
    },
    BackupFailed {
        message: String,
    },
    RestoreValidationFailed {
        message: String,
    },
    OutboxAppendFailed {
        message: String,
    },
    OutboxDeliveryFailed {
        message: String,
    },
    OutboxRecordCorrupt {
        message: String,
    },
    JournalAppendFailed {
        message: String,
    },
    JournalReplayFailed {
        message: String,
    },
    JournalCursorInvalid {
        message: String,
    },
    BlobWriteFailed {
        message: String,
    },
    BlobCorrupt {
        id: BlobId,
    },
    BlobNotFound {
        id: BlobId,
    },
    JobStoreFailure {
        message: String,
    },
    JobInterrupted {
        job: JobId,
    },
    UnsafeAutomaticRecovery {
        job: JobId,
    },
    InvalidRecord {
        message: String,
    },
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StorageOpenFailed { message } => {
                write!(formatter, "storage open failed: {message}")
            }
            Self::UnsupportedStorageFormat { found, supported } => write!(
                formatter,
                "storage format {found} is newer than supported format {supported}"
            ),
            Self::StorageCorrupt { message } => write!(formatter, "storage is corrupt: {message}"),
            Self::StorageIoFailure { operation, message } => {
                write!(
                    formatter,
                    "storage I/O failed during {operation}: {message}"
                )
            }
            Self::StorageBusy { message } => write!(formatter, "storage is busy: {message}"),
            Self::DurabilityFailure { message } => {
                write!(formatter, "storage durability failed: {message}")
            }
            Self::PersistentCasConflict { message } => {
                write!(formatter, "persistent CAS conflict: {message}")
            }
            Self::BackupFailed { message } => write!(formatter, "backup failed: {message}"),
            Self::RestoreValidationFailed { message } => {
                write!(formatter, "restore validation failed: {message}")
            }
            Self::OutboxAppendFailed { message } => {
                write!(formatter, "outbox append failed: {message}")
            }
            Self::OutboxDeliveryFailed { message } => {
                write!(formatter, "outbox delivery failed: {message}")
            }
            Self::OutboxRecordCorrupt { message } => {
                write!(formatter, "outbox record is corrupt: {message}")
            }
            Self::JournalAppendFailed { message } => {
                write!(formatter, "journal append failed: {message}")
            }
            Self::JournalReplayFailed { message } => {
                write!(formatter, "journal replay failed: {message}")
            }
            Self::JournalCursorInvalid { message } => {
                write!(formatter, "journal cursor is invalid: {message}")
            }
            Self::BlobWriteFailed { message } => write!(formatter, "blob write failed: {message}"),
            Self::BlobCorrupt { id } => write!(formatter, "blob '{id}' is corrupt"),
            Self::BlobNotFound { id } => write!(formatter, "blob '{id}' was not found"),
            Self::JobStoreFailure { message } => write!(formatter, "job store failed: {message}"),
            Self::JobInterrupted { job } => write!(formatter, "job '{job}' was interrupted"),
            Self::UnsafeAutomaticRecovery { job } => {
                write!(formatter, "automatic recovery of job '{job}' is unsafe")
            }
            Self::InvalidRecord { message } => {
                write!(formatter, "invalid persistence record: {message}")
            }
        }
    }
}

impl std::error::Error for PersistenceError {}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OutboxId(String);

impl OutboxId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for OutboxId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for OutboxId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for OutboxId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum OutboxStatus {
    #[default]
    Pending,
    Delivering,
    Delivered,
    Failed,
}

impl fmt::Display for OutboxStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pending => "Pending",
            Self::Delivering => "Delivering",
            Self::Delivered => "Delivered",
            Self::Failed => "Failed",
        })
    }
}

/// A previously admitted notification intent. The event envelope retains the
/// trusted producer metadata and logical EventId across redelivery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxRecord {
    outbox_id: OutboxId,
    installation: InstallationId,
    event: EventEnvelope,
    status: OutboxStatus,
    attempt_count: u32,
    created_sequence: u64,
    created_at_millis: u64,
}

impl OutboxRecord {
    pub fn new(
        outbox_id: impl Into<OutboxId>,
        installation: InstallationId,
        event: EventEnvelope,
        created_sequence: u64,
        created_at_millis: u64,
    ) -> Self {
        Self {
            outbox_id: outbox_id.into(),
            installation,
            event,
            status: OutboxStatus::Pending,
            attempt_count: 0,
            created_sequence,
            created_at_millis,
        }
    }

    pub fn outbox_id(&self) -> &OutboxId {
        &self.outbox_id
    }

    pub fn installation(&self) -> &InstallationId {
        &self.installation
    }

    pub fn event(&self) -> &EventEnvelope {
        &self.event
    }

    pub const fn status(&self) -> OutboxStatus {
        self.status
    }

    pub const fn attempt_count(&self) -> u32 {
        self.attempt_count
    }

    pub const fn created_sequence(&self) -> u64 {
        self.created_sequence
    }

    pub const fn created_at_millis(&self) -> u64 {
        self.created_at_millis
    }

    pub fn with_status(&self, status: OutboxStatus, attempt_count: u32) -> Self {
        let mut record = self.clone();
        record.status = status;
        record.attempt_count = attempt_count;
        record
    }
}

pub trait OutboxStore: Send + Sync {
    fn append(&self, record: OutboxRecord) -> Result<(), PersistenceError>;
    fn get(&self, id: &OutboxId) -> Result<Option<OutboxRecord>, PersistenceError>;
    fn list_pending(&self, limit: usize) -> Result<Vec<OutboxRecord>, PersistenceError>;
    fn mark_delivering(&self, id: &OutboxId) -> Result<OutboxRecord, PersistenceError>;
    fn mark_delivered(&self, id: &OutboxId) -> Result<(), PersistenceError>;
    fn mark_failed(&self, id: &OutboxId, message: &str) -> Result<(), PersistenceError>;
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AuditOutcome {
    Committed,
    NotCommitted,
    PendingDelivery,
    Interrupted,
    Failed,
}

impl fmt::Display for AuditOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Committed => "Committed",
            Self::NotCommitted => "NotCommitted",
            Self::PendingDelivery => "PendingDelivery",
            Self::Interrupted => "Interrupted",
            Self::Failed => "Failed",
        })
    }
}

/// Safe metadata-only audit record. There is deliberately no raw payload or
/// raw state field in this contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditRecord {
    audit_id: u64,
    record_type: String,
    principal: Option<PrincipalId>,
    installation: Option<InstallationId>,
    runtime: Option<crate::RuntimeId>,
    correlation: Option<CorrelationId>,
    causation: Option<CausationRef>,
    outcome: AuditOutcome,
    metadata: BTreeMap<String, String>,
}

impl AuditRecord {
    pub fn new(
        audit_id: u64,
        record_type: impl Into<String>,
        outcome: AuditOutcome,
        metadata: BTreeMap<String, String>,
    ) -> Self {
        Self {
            audit_id,
            record_type: record_type.into(),
            principal: None,
            installation: None,
            runtime: None,
            correlation: None,
            causation: None,
            outcome,
            metadata,
        }
    }

    pub fn with_principal(mut self, principal: PrincipalId) -> Self {
        self.principal = Some(principal);
        self
    }

    pub fn with_installation(mut self, installation: InstallationId) -> Self {
        self.installation = Some(installation);
        self
    }

    pub fn with_runtime(mut self, runtime: crate::RuntimeId) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Reconstructs the runtime metadata recorded by a trusted durable store.
    /// This does not make the runtime live or restore its authority.
    pub fn with_runtime_parts(mut self, incarnation: u64, sequence: u64) -> Self {
        self.runtime = Some(crate::RuntimeId::new(incarnation, sequence));
        self
    }

    pub fn with_trace(
        mut self,
        correlation: CorrelationId,
        causation: Option<CausationRef>,
    ) -> Self {
        self.correlation = Some(correlation);
        self.causation = causation;
        self
    }

    pub const fn audit_id(&self) -> u64 {
        self.audit_id
    }

    pub fn record_type(&self) -> &str {
        &self.record_type
    }

    pub fn principal(&self) -> Option<&PrincipalId> {
        self.principal.as_ref()
    }

    pub fn installation(&self) -> Option<&InstallationId> {
        self.installation.as_ref()
    }

    pub const fn runtime(&self) -> Option<crate::RuntimeId> {
        self.runtime
    }

    pub fn correlation(&self) -> Option<&CorrelationId> {
        self.correlation.as_ref()
    }

    pub fn causation(&self) -> Option<&CausationRef> {
        self.causation.as_ref()
    }

    pub const fn outcome(&self) -> AuditOutcome {
        self.outcome
    }

    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }
}

pub trait AuditStore: Send + Sync {
    fn append(&self, record: AuditRecord) -> Result<(), PersistenceError>;
    fn list(&self, limit: usize) -> Result<Vec<AuditRecord>, PersistenceError>;
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlobId(String);

impl BlobId {
    pub fn new(value: impl Into<String>) -> Result<Self, PersistenceError> {
        let value = value.into();
        let valid = value.strip_prefix("sha256-v1-").is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        if valid {
            Ok(Self(value))
        } else {
            Err(PersistenceError::InvalidRecord {
                message: "blob identity must be sha256-v1- followed by 64 hex digits".to_owned(),
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub trait BlobStore: Send + Sync {
    fn put(&self, bytes: &[u8]) -> Result<BlobId, PersistenceError>;
    fn get(&self, id: &BlobId) -> Result<Vec<u8>, PersistenceError>;
    fn exists(&self, id: &BlobId) -> Result<bool, PersistenceError>;
    fn verify(&self, id: &BlobId) -> Result<(), PersistenceError>;
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JobId(String);

impl JobId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for JobId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for JobId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum JobState {
    Pending,
    Waiting,
    Runnable,
    Running,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

impl fmt::Display for JobState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pending => "Pending",
            Self::Waiting => "Waiting",
            Self::Runnable => "Runnable",
            Self::Running => "Running",
            Self::Completed => "Completed",
            Self::Cancelled => "Cancelled",
            Self::Failed => "Failed",
            Self::Interrupted => "Interrupted",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum JobRecoveryPolicy {
    #[default]
    Manual,
    RetrySafe,
    Idempotent,
}

impl fmt::Display for JobRecoveryPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Manual => "Manual",
            Self::RetrySafe => "RetrySafe",
            Self::Idempotent => "Idempotent",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobRecord {
    job_id: JobId,
    owner: PrincipalId,
    installation: Option<InstallationId>,
    state: JobState,
    deadline_millis: Option<u64>,
    wakeup_millis: Option<u64>,
    cancellation_requested: bool,
    attempt: u32,
    resource_budget: Option<u64>,
    recovery_policy: JobRecoveryPolicy,
    correlation: Option<CorrelationId>,
    causation: Option<CausationRef>,
}

impl JobRecord {
    pub fn new(job_id: impl Into<JobId>, owner: PrincipalId) -> Self {
        Self {
            job_id: job_id.into(),
            owner,
            installation: None,
            state: JobState::Pending,
            deadline_millis: None,
            wakeup_millis: None,
            cancellation_requested: false,
            attempt: 0,
            resource_budget: None,
            recovery_policy: JobRecoveryPolicy::Manual,
            correlation: None,
            causation: None,
        }
    }

    pub fn with_installation(mut self, installation: InstallationId) -> Self {
        self.installation = Some(installation);
        self
    }

    pub fn with_state(mut self, state: JobState) -> Self {
        self.state = state;
        self
    }

    pub fn with_deadline_millis(mut self, deadline: Option<u64>) -> Self {
        self.deadline_millis = deadline;
        self
    }

    pub fn with_wakeup_millis(mut self, wakeup: Option<u64>) -> Self {
        self.wakeup_millis = wakeup;
        self
    }

    pub fn with_cancellation_requested(mut self, requested: bool) -> Self {
        self.cancellation_requested = requested;
        self
    }

    pub fn with_attempt(mut self, attempt: u32) -> Self {
        self.attempt = attempt;
        self
    }

    pub fn with_resource_budget(mut self, budget: Option<u64>) -> Self {
        self.resource_budget = budget;
        self
    }

    pub fn with_recovery_policy(mut self, policy: JobRecoveryPolicy) -> Self {
        self.recovery_policy = policy;
        self
    }

    pub fn with_trace(
        mut self,
        correlation: CorrelationId,
        causation: Option<CausationRef>,
    ) -> Self {
        self.correlation = Some(correlation);
        self.causation = causation;
        self
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn owner(&self) -> &PrincipalId {
        &self.owner
    }

    pub fn installation(&self) -> Option<&InstallationId> {
        self.installation.as_ref()
    }

    pub const fn state(&self) -> JobState {
        self.state
    }

    pub const fn deadline_millis(&self) -> Option<u64> {
        self.deadline_millis
    }

    pub const fn wakeup_millis(&self) -> Option<u64> {
        self.wakeup_millis
    }

    pub const fn cancellation_requested(&self) -> bool {
        self.cancellation_requested
    }

    pub const fn attempt(&self) -> u32 {
        self.attempt
    }

    pub const fn resource_budget(&self) -> Option<u64> {
        self.resource_budget
    }

    pub const fn recovery_policy(&self) -> JobRecoveryPolicy {
        self.recovery_policy
    }

    pub fn correlation(&self) -> Option<&CorrelationId> {
        self.correlation.as_ref()
    }

    pub fn causation(&self) -> Option<&CausationRef> {
        self.causation.as_ref()
    }
}

pub trait JobStore: Send + Sync {
    fn create(&self, job: JobRecord) -> Result<(), PersistenceError>;
    fn get(&self, id: &JobId) -> Result<Option<JobRecord>, PersistenceError>;
    fn list(&self) -> Result<Vec<JobRecord>, PersistenceError>;
    fn update(&self, job: JobRecord) -> Result<(), PersistenceError>;
    fn recover_running(&self) -> Result<Vec<JobRecord>, PersistenceError>;
}
