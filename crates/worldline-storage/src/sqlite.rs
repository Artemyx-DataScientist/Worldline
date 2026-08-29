use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior, params,
    types::Type,
};
use sha2::{Digest, Sha256};
use worldline_kernel::{
    AuditOutcome, AuditRecord, AuditStore, BackendState, CURRENT_STORAGE_FORMAT_VERSION,
    CausationRef, CorrelationId, DeliveryMode, EventContract, EventCursor, EventEnvelope, EventId,
    EventJournal, EventJournalError, InstallationId, InstallationRecord, InstallationStatus,
    InterfaceVersion, InvocationCompletedMetadata, InvocationId, JobId, JobRecord,
    JobRecoveryPolicy, JobState, JobStore, OperationId, OutboxId, OutboxRecord, OutboxStatus,
    OutboxStore, PersistenceError, PluginId, PrincipalId, RpcOutcomeClass, RpcRequestId, RuntimeId,
    StateBackend, StateError, StateKey, StateRevision, StateSchemaVersion, StateTransactionId,
    StorageFormatVersion,
};

const DATABASE_FILE: &str = "worldline.sqlite3";
const STORAGE_FORMAT_KEY: &str = "format_version";
const JOURNAL_MODE: &str = "WAL";
const EVENT_CODEC_MAGIC: &[u8] = b"WL-EVENT-1";
const MAX_CODEC_STRING_BYTES: usize = 1 << 20;
const MAX_CODEC_PAYLOAD_BYTES: usize = 16 << 20;
static NEXT_BACKUP_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

type RecordColumns = (String, String, i64, String, i64, i64);
type OutboxColumns = (String, String, String, String, i64, i64, i64, Vec<u8>);
type AuditColumns = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Vec<u8>,
);
type JobColumns = (
    String,
    String,
    Option<String>,
    String,
    Option<i64>,
    Option<i64>,
    i64,
    i64,
    Option<i64>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// SQLite-backed production StateBackend.
///
/// The connection and schema are private to this host-side crate. The kernel
/// sees only the StateBackend contract and opaque state values.
pub struct SqliteStateBackend {
    profile_root: PathBuf,
    connection: Mutex<Connection>,
}

impl SqliteStateBackend {
    pub fn open(profile_root: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let profile_root = canonical_profile_root(profile_root.as_ref())?;
        let connection = open_profile_connection(&profile_root)?;
        let backend = Self {
            profile_root,
            connection: Mutex::new(connection),
        };
        Ok(backend)
    }

    pub fn profile_root(&self) -> &Path {
        &self.profile_root
    }

    pub fn database_path(&self) -> PathBuf {
        self.profile_root.join(DATABASE_FILE)
    }

    /// Creates a transactionally consistent SQLite online-backup artifact.
    ///
    /// The destination must not already exist. This avoids silently replacing
    /// an operator-selected backup and makes the publication boundary
    /// explicit. Blob files are a separate store and are not implicitly
    /// included in this metadata backup.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<(), PersistenceError> {
        let destination = destination.as_ref();
        if destination.exists() {
            return Err(PersistenceError::BackupFailed {
                message: format!(
                    "backup destination '{}' already exists",
                    destination.display()
                ),
            });
        }
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|error| PersistenceError::BackupFailed {
            message: format!(
                "cannot create backup destination directory '{}': {error}",
                parent.display()
            ),
        })?;
        let sequence = NEXT_BACKUP_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let file_name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("worldline-backup");
        let temporary = parent.join(format!(
            ".{file_name}.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        let result = (|| {
            let source = self.connection().map_err(state_error_to_persistence)?;
            let mut target =
                Connection::open(&temporary).map_err(|error| PersistenceError::BackupFailed {
                    message: format!("open temporary backup '{}': {error}", temporary.display()),
                })?;
            {
                let backup =
                    rusqlite::backup::Backup::new(&source, &mut target).map_err(|error| {
                        PersistenceError::BackupFailed {
                            message: format!("start online backup: {error}"),
                        }
                    })?;
                backup
                    .run_to_completion(16, std::time::Duration::from_millis(10), None)
                    .map_err(|error| PersistenceError::BackupFailed {
                        message: format!("copy online backup pages: {error}"),
                    })?;
            }
            drop(target);
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&temporary)
                .and_then(|file| file.sync_all())
                .map_err(|error| PersistenceError::BackupFailed {
                    message: format!("sync backup '{}': {error}", temporary.display()),
                })?;
            fs::rename(&temporary, destination).map_err(|error| {
                PersistenceError::BackupFailed {
                    message: format!(
                        "publish backup '{}' as '{}': {error}",
                        temporary.display(),
                        destination.display()
                    ),
                }
            })?;
            Self::validate_backup(destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    /// Validates an existing backup without opening it as a live profile.
    pub fn validate_backup(source: impl AsRef<Path>) -> Result<(), PersistenceError> {
        let source = source.as_ref();
        if !source.is_file() {
            return Err(PersistenceError::RestoreValidationFailed {
                message: format!("backup '{}' is not a file", source.display()),
            });
        }
        let connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| PersistenceError::RestoreValidationFailed {
                message: format!("cannot open backup '{}': {error}", source.display()),
            })?;
        validate_storage_schema(&connection)
    }

    /// Restores a validated metadata backup into a fresh profile root and
    /// reopens it through the production StateBackend constructor.
    pub fn restore_from(
        source: impl AsRef<Path>,
        profile_root: impl AsRef<Path>,
    ) -> Result<Self, PersistenceError> {
        let source = source.as_ref();
        Self::validate_backup(source)?;
        let profile_root = canonical_profile_root(profile_root.as_ref())?;
        let destination = profile_root.join(DATABASE_FILE);
        if destination.exists() {
            return Err(PersistenceError::RestoreValidationFailed {
                message: format!(
                    "restore destination '{}' is not a fresh profile",
                    profile_root.display()
                ),
            });
        }
        let sequence = NEXT_BACKUP_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary = profile_root.join(format!(".{}.{}.restore.tmp", DATABASE_FILE, sequence));
        let result = (|| {
            fs::copy(source, &temporary).map_err(|error| {
                PersistenceError::RestoreValidationFailed {
                    message: format!("copy backup into restore profile: {error}"),
                }
            })?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&temporary)
                .and_then(|file| file.sync_all())
                .map_err(|error| PersistenceError::RestoreValidationFailed {
                    message: format!("sync restored database: {error}"),
                })?;
            fs::rename(&temporary, &destination).map_err(|error| {
                PersistenceError::RestoreValidationFailed {
                    message: format!("publish restored database: {error}"),
                }
            })?;
            Self::open(&profile_root).map_err(|error| PersistenceError::RestoreValidationFailed {
                message: format!("reopened restore failed validation: {error}"),
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_file(&destination);
        }
        result
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StateError> {
        self.connection.lock().map_err(|_| {
            StateError::Persistence(PersistenceError::StorageIoFailure {
                operation: "lock".to_owned(),
                message: "storage connection mutex is poisoned".to_owned(),
            })
        })
    }

    fn transaction<'a>(connection: &'a mut Connection) -> Result<Transaction<'a>, StateError> {
        connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| map_sqlite_error("begin transaction", error))
    }

    fn read_record(
        connection: &Connection,
        installation: &InstallationId,
    ) -> Result<Option<InstallationRecord>, StateError> {
        let columns = connection
            .query_row(
                "SELECT installation_id, plugin_id, state_schema_version, status, revision, runtime_generation
                 FROM installations WHERE installation_id = ?1",
                params![installation.as_str()],
                record_columns,
            )
            .optional()
            .map_err(|error| map_sqlite_error("read installation record", error))?;
        columns.map(record_from_columns).transpose()
    }

    fn read_values(
        connection: &Connection,
        installation: &InstallationId,
    ) -> Result<BTreeMap<StateKey, Vec<u8>>, StateError> {
        let mut statement = connection
            .prepare(
                "SELECT state_key, state_value
                 FROM state_entries WHERE installation_id = ?1 ORDER BY state_key",
            )
            .map_err(|error| map_sqlite_error("prepare state listing", error))?;
        let rows = statement
            .query_map(params![installation.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(|error| map_sqlite_error("list state entries", error))?;
        let mut values = BTreeMap::new();
        for row in rows {
            let (key, value) = row.map_err(|error| map_sqlite_error("read state entry", error))?;
            values.insert(StateKey::new(key), value);
        }
        Ok(values)
    }
}

fn canonical_profile_root(requested_root: &Path) -> Result<PathBuf, PersistenceError> {
    fs::create_dir_all(requested_root).map_err(|error| PersistenceError::StorageOpenFailed {
        message: format!(
            "cannot create profile root '{}': {error}",
            requested_root.display()
        ),
    })?;
    fs::canonicalize(requested_root).map_err(|error| PersistenceError::StorageOpenFailed {
        message: format!(
            "cannot canonicalize profile root '{}': {error}",
            requested_root.display()
        ),
    })
}

fn open_profile_connection(profile_root: &Path) -> Result<Connection, PersistenceError> {
    let database_path = profile_root.join(DATABASE_FILE);
    let connection =
        Connection::open(&database_path).map_err(|error| PersistenceError::StorageOpenFailed {
            message: format!("cannot open '{}': {error}", database_path.display()),
        })?;
    configure_connection(&connection)?;
    initialize_schema(&connection)?;
    Ok(connection)
}

fn configure_connection(connection: &Connection) -> Result<(), PersistenceError> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| open_error("enable foreign keys", error))?;
    connection
        .pragma_update(None, "journal_mode", JOURNAL_MODE)
        .map_err(|error| open_error("enable WAL journal mode", error))?;
    let journal_mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|error| open_error("confirm WAL journal mode", error))?;
    if !journal_mode.eq_ignore_ascii_case(JOURNAL_MODE) {
        return Err(PersistenceError::StorageOpenFailed {
            message: format!(
                "SQLite requested journal mode '{JOURNAL_MODE}', got '{journal_mode}'"
            ),
        });
    }
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| open_error("enable FULL synchronous mode", error))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|error| open_error("configure busy timeout", error))?;
    Ok(())
}

fn initialize_schema(connection: &Connection) -> Result<(), PersistenceError> {
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS worldline_storage_meta (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS installations (
                installation_id TEXT PRIMARY KEY NOT NULL,
                plugin_id TEXT NOT NULL,
                state_schema_version INTEGER NOT NULL,
                status TEXT NOT NULL,
                revision INTEGER NOT NULL,
                runtime_generation INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS state_entries (
                installation_id TEXT NOT NULL,
                state_key TEXT NOT NULL,
                state_value BLOB NOT NULL,
                PRIMARY KEY (installation_id, state_key),
                FOREIGN KEY (installation_id)
                    REFERENCES installations(installation_id)
                    ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS outbox (
                outbox_id TEXT PRIMARY KEY NOT NULL,
                installation_id TEXT NOT NULL,
                event_id TEXT NOT NULL,
                record_status TEXT NOT NULL,
                attempt_count INTEGER NOT NULL,
                created_sequence INTEGER NOT NULL,
                created_at_millis INTEGER NOT NULL,
                record BLOB NOT NULL,
                failure_message TEXT,
                FOREIGN KEY (installation_id)
                    REFERENCES installations(installation_id)
                    ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS event_journal (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT UNIQUE NOT NULL,
                record BLOB NOT NULL,
                record_hash BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS audit_records (
                audit_id INTEGER PRIMARY KEY NOT NULL,
                record_type TEXT NOT NULL,
                principal TEXT,
                installation_id TEXT,
                runtime_incarnation INTEGER,
                runtime_sequence INTEGER,
                correlation_id TEXT,
                causation_kind TEXT,
                causation_id TEXT,
                outcome TEXT NOT NULL,
                metadata BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS jobs (
                job_id TEXT PRIMARY KEY NOT NULL,
                owner TEXT NOT NULL,
                installation_id TEXT,
                state TEXT NOT NULL,
                deadline_millis INTEGER,
                wakeup_millis INTEGER,
                cancellation_requested INTEGER NOT NULL,
                attempt INTEGER NOT NULL,
                resource_budget INTEGER,
                recovery_policy TEXT NOT NULL,
                correlation_id TEXT,
                causation_kind TEXT,
                causation_id TEXT
            );
            ",
        )
        .map_err(|error| open_error("initialize schema", error))?;

    // Databases created by the first T-003 implementation have the same
    // storage format but lack this diagnostic column. Adding a nullable
    // column is non-destructive and keeps those profiles reopenable.
    let has_failure_message = connection
        .prepare("PRAGMA table_info(outbox)")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .try_fold(false, |found, name| Ok(found || name? == "failure_message"))
        })
        .map_err(|error| open_error("inspect outbox schema", error))?;
    if !has_failure_message {
        connection
            .execute("ALTER TABLE outbox ADD COLUMN failure_message TEXT", [])
            .map_err(|error| open_error("extend outbox schema", error))?;
    }

    let existing: Option<String> = connection
        .query_row(
            "SELECT value FROM worldline_storage_meta WHERE key = ?1",
            params![STORAGE_FORMAT_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| open_error("read storage format", error))?;
    match existing {
        Some(value) => {
            let found = value
                .parse::<u32>()
                .map_err(|_| PersistenceError::StorageCorrupt {
                    message: format!("storage format value '{value}' is not an integer"),
                })?;
            let found = StorageFormatVersion::new(found);
            if found > CURRENT_STORAGE_FORMAT_VERSION {
                return Err(PersistenceError::UnsupportedStorageFormat {
                    found,
                    supported: CURRENT_STORAGE_FORMAT_VERSION,
                });
            }
            if found < CURRENT_STORAGE_FORMAT_VERSION {
                return Err(PersistenceError::StorageCorrupt {
                    message: format!(
                        "storage format {found} has no non-destructive migration to {}",
                        CURRENT_STORAGE_FORMAT_VERSION
                    ),
                });
            }
        }
        None => {
            connection
                .execute(
                    "INSERT INTO worldline_storage_meta(key, value) VALUES (?1, ?2)",
                    params![
                        STORAGE_FORMAT_KEY,
                        CURRENT_STORAGE_FORMAT_VERSION.value().to_string()
                    ],
                )
                .map_err(|error| open_error("write storage format", error))?;
        }
    }
    Ok(())
}

fn validate_storage_schema(connection: &Connection) -> Result<(), PersistenceError> {
    let required_tables = [
        "worldline_storage_meta",
        "installations",
        "state_entries",
        "outbox",
        "event_journal",
        "audit_records",
        "jobs",
    ];
    for table in required_tables {
        let exists: Option<String> = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| PersistenceError::RestoreValidationFailed {
                message: format!("cannot inspect required table '{table}': {error}"),
            })?;
        if exists.is_none() {
            return Err(PersistenceError::RestoreValidationFailed {
                message: format!("required table '{table}' is missing"),
            });
        }
    }
    let format: String = connection
        .query_row(
            "SELECT value FROM worldline_storage_meta WHERE key = ?1",
            params![STORAGE_FORMAT_KEY],
            |row| row.get(0),
        )
        .map_err(|error| PersistenceError::RestoreValidationFailed {
            message: format!("cannot read backup storage format: {error}"),
        })?;
    let found = format
        .parse::<u32>()
        .map_err(|_| PersistenceError::RestoreValidationFailed {
            message: format!("backup storage format '{format}' is not an integer"),
        })?;
    let found = StorageFormatVersion::new(found);
    if found != CURRENT_STORAGE_FORMAT_VERSION {
        return if found > CURRENT_STORAGE_FORMAT_VERSION {
            Err(PersistenceError::UnsupportedStorageFormat {
                found,
                supported: CURRENT_STORAGE_FORMAT_VERSION,
            })
        } else {
            Err(PersistenceError::RestoreValidationFailed {
                message: format!(
                    "backup storage format {found} does not match supported format {}",
                    CURRENT_STORAGE_FORMAT_VERSION
                ),
            })
        };
    }
    Ok(())
}

impl StateBackend for SqliteStateBackend {
    fn create(&self, state: BackendState) -> Result<(), StateError> {
        let installation = state.record().installation_id().clone();
        let mut connection = self.connection()?;
        let transaction = Self::transaction(&mut connection)?;
        let exists: Option<String> = transaction
            .query_row(
                "SELECT installation_id FROM installations WHERE installation_id = ?1",
                params![installation.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| map_sqlite_error("check installation", error))?;
        if exists.is_some() {
            return Err(StateError::InstallationAlreadyExists { installation });
        }
        insert_state(&transaction, &state)?;
        transaction
            .commit()
            .map_err(|error| map_sqlite_error("commit installation creation", error))
    }

    fn snapshot(&self, installation: &InstallationId) -> Result<BackendState, StateError> {
        let connection = self.connection()?;
        let record = Self::read_record(&connection, installation)?.ok_or_else(|| {
            StateError::UnknownInstallation {
                installation: installation.clone(),
            }
        })?;
        let values = Self::read_values(&connection, installation)?;
        Ok(BackendState::new(record, values))
    }

    fn commit_if_revision(
        &self,
        installation: &InstallationId,
        transaction_id: &StateTransactionId,
        expected_revision: StateRevision,
        state: BackendState,
    ) -> Result<(), StateError> {
        if state.record().installation_id() != installation {
            return Err(StateError::BackendFailure {
                operation: "commit",
                message: "state installation does not match commit target".to_owned(),
            });
        }
        let expected_next =
            expected_revision
                .value()
                .checked_add(1)
                .ok_or_else(|| StateError::BackendFailure {
                    operation: "commit",
                    message: "state revision counter exhausted".to_owned(),
                })?;
        if state.record().state_revision().value() != expected_next {
            return Err(StateError::BackendFailure {
                operation: "commit",
                message: format!(
                    "commit revision must advance from {expected_revision} to {expected_next}, got {}",
                    state.record().state_revision()
                ),
            });
        }
        let mut connection = self.connection()?;
        let transaction = Self::transaction(&mut connection)?;
        let actual = current_revision(&transaction, installation)?;
        if actual != expected_revision {
            return Err(StateError::TransactionConflict {
                installation: installation.clone(),
                transaction: transaction_id.clone(),
                expected_revision,
                actual_revision: actual,
            });
        }
        replace_state(&transaction, &state)?;
        crate::failpoints::hit("before-state-commit");
        let result = transaction
            .commit()
            .map_err(|error| map_sqlite_error("commit state transaction", error));
        if result.is_ok() {
            crate::failpoints::hit("after-state-commit");
        }
        result
    }

    fn commit_if_revision_with_outbox(
        &self,
        installation: &InstallationId,
        transaction_id: &StateTransactionId,
        expected_revision: StateRevision,
        state: BackendState,
        outbox: &OutboxRecord,
    ) -> Result<(), StateError> {
        validate_outbox_for_installation(installation, outbox)?;
        if state.record().installation_id() != installation {
            return Err(StateError::BackendFailure {
                operation: "commit",
                message: "state installation does not match commit target".to_owned(),
            });
        }
        let expected_next =
            expected_revision
                .value()
                .checked_add(1)
                .ok_or_else(|| StateError::BackendFailure {
                    operation: "commit",
                    message: "state revision counter exhausted".to_owned(),
                })?;
        if state.record().state_revision().value() != expected_next {
            return Err(StateError::BackendFailure {
                operation: "commit",
                message: format!(
                    "commit revision must advance from {expected_revision} to {expected_next}, got {}",
                    state.record().state_revision()
                ),
            });
        }
        let encoded_event = encode_event(outbox.event()).map_err(|message| {
            StateError::Persistence(PersistenceError::OutboxRecordCorrupt { message })
        })?;
        let mut connection = self.connection()?;
        let transaction = Self::transaction(&mut connection)?;
        let actual = current_revision(&transaction, installation)?;
        if actual != expected_revision {
            return Err(StateError::TransactionConflict {
                installation: installation.clone(),
                transaction: transaction_id.clone(),
                expected_revision,
                actual_revision: actual,
            });
        }
        replace_state(&transaction, &state)?;
        insert_outbox(&transaction, outbox, &encoded_event)?;
        crate::failpoints::hit("after-state-outbox-before-commit");
        let result = transaction
            .commit()
            .map_err(|error| map_sqlite_error("commit state and outbox transaction", error));
        if result.is_ok() {
            crate::failpoints::hit("after-state-outbox-commit-before-publish");
        }
        result
    }

    fn update_record_if_revision(
        &self,
        expected_revision: StateRevision,
        record: InstallationRecord,
    ) -> Result<(), StateError> {
        let installation = record.installation_id().clone();
        let mut connection = self.connection()?;
        let transaction = Self::transaction(&mut connection)?;
        let actual = current_revision(&transaction, &installation)?;
        if actual != expected_revision {
            return Err(StateError::RevisionConflict {
                installation,
                expected_revision,
                actual_revision: actual,
            });
        }
        let updated = transaction
            .execute(
                "UPDATE installations
                 SET plugin_id = ?1, state_schema_version = ?2, status = ?3,
                     revision = ?4, runtime_generation = ?5
                 WHERE installation_id = ?6 AND revision = ?7",
                params![
                    record.plugin_id().as_str(),
                    integer(record.state_schema_version().value(), "schema version")?,
                    record.status().to_string(),
                    integer(record.state_revision().value(), "record revision")?,
                    integer(record.runtime_generation(), "runtime generation")?,
                    installation.as_str(),
                    integer(expected_revision.value(), "expected revision")?,
                ],
            )
            .map_err(|error| map_sqlite_error("update installation record", error))?;
        if updated != 1 {
            return Err(StateError::RevisionConflict {
                installation,
                expected_revision,
                actual_revision: current_revision(&transaction, record.installation_id())?,
            });
        }
        let result = transaction
            .commit()
            .map_err(|error| map_sqlite_error("commit metadata transition", error));
        if result.is_ok() {
            match record.status() {
                InstallationStatus::Migrating => crate::failpoints::hit("during-migration"),
                InstallationStatus::Uninstalling => {
                    crate::failpoints::hit("during-uninstall-metadata-transition")
                }
                _ => {}
            }
        }
        result
    }

    fn delete(&self, installation: &InstallationId) -> Result<(), StateError> {
        let mut connection = self.connection()?;
        let transaction = Self::transaction(&mut connection)?;
        let deleted = transaction
            .execute(
                "DELETE FROM installations WHERE installation_id = ?1",
                params![installation.as_str()],
            )
            .map_err(|error| map_sqlite_error("delete installation", error))?;
        if deleted != 1 {
            return Err(StateError::UnknownInstallation {
                installation: installation.clone(),
            });
        }
        transaction
            .commit()
            .map_err(|error| map_sqlite_error("commit installation deletion", error))
    }

    fn list_records(&self) -> Result<Vec<InstallationRecord>, StateError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT installation_id, plugin_id, state_schema_version, status, revision, runtime_generation
                 FROM installations ORDER BY installation_id",
            )
            .map_err(|error| map_sqlite_error("prepare installation listing", error))?;
        let rows = statement
            .query_map([], record_columns)
            .map_err(|error| map_sqlite_error("list installations", error))?;
        rows.map(|row| {
            row.map_err(|error| map_sqlite_error("read installation listing", error))
                .and_then(record_from_columns)
        })
        .collect()
    }
}

impl OutboxStore for SqliteStateBackend {
    fn append(&self, record: OutboxRecord) -> Result<(), PersistenceError> {
        validate_outbox_for_installation(record.installation(), &record)
            .map_err(state_error_to_persistence)?;
        let encoded_event = encode_event(record.event())
            .map_err(|message| PersistenceError::OutboxRecordCorrupt { message })?;
        let mut connection =
            self.connection
                .lock()
                .map_err(|_| PersistenceError::StorageIoFailure {
                    operation: "lock".to_owned(),
                    message: "storage connection mutex is poisoned".to_owned(),
                })?;
        let transaction = transaction_for_persistence(&mut connection, "begin outbox append")?;
        let installation_exists: Option<String> = transaction
            .query_row(
                "SELECT installation_id FROM installations WHERE installation_id = ?1",
                params![record.installation().as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| outbox_sqlite_error("check outbox installation", error))?;
        if installation_exists.is_none() {
            return Err(PersistenceError::OutboxAppendFailed {
                message: format!("installation '{}' does not exist", record.installation()),
            });
        }
        let already_exists: Option<String> = transaction
            .query_row(
                "SELECT outbox_id FROM outbox WHERE outbox_id = ?1",
                params![record.outbox_id().as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| outbox_sqlite_error("check outbox identity", error))?;
        if already_exists.is_some() {
            return Err(PersistenceError::OutboxAppendFailed {
                message: format!("outbox '{}' already exists", record.outbox_id()),
            });
        }
        insert_outbox(&transaction, &record, &encoded_event).map_err(state_error_to_persistence)?;
        transaction
            .commit()
            .map_err(|error| outbox_sqlite_error("commit outbox append", error))
    }

    fn get(&self, id: &OutboxId) -> Result<Option<OutboxRecord>, PersistenceError> {
        let connection =
            self.connection
                .lock()
                .map_err(|_| PersistenceError::StorageIoFailure {
                    operation: "lock".to_owned(),
                    message: "storage connection mutex is poisoned".to_owned(),
                })?;
        let columns = read_outbox_columns(&connection, id)
            .map_err(|error| outbox_sqlite_error("read outbox record", error))?;
        columns.map(decode_outbox_columns).transpose()
    }

    fn list_pending(&self, limit: usize) -> Result<Vec<OutboxRecord>, PersistenceError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).map_err(|_| PersistenceError::OutboxDeliveryFailed {
            message: "outbox listing limit exceeds SQLite integer range".to_owned(),
        })?;
        let connection =
            self.connection
                .lock()
                .map_err(|_| PersistenceError::StorageIoFailure {
                    operation: "lock".to_owned(),
                    message: "storage connection mutex is poisoned".to_owned(),
                })?;
        let mut statement = connection
            .prepare(
                "SELECT outbox_id, installation_id, event_id, record_status,
                        attempt_count, created_sequence, created_at_millis, record
                 FROM outbox
                 WHERE record_status IN ('Pending', 'Delivering')
                 ORDER BY created_sequence, outbox_id
                 LIMIT ?1",
            )
            .map_err(|error| outbox_sqlite_error("prepare outbox listing", error))?;
        let rows = statement
            .query_map(params![limit], outbox_columns)
            .map_err(|error| outbox_sqlite_error("list pending outbox", error))?;
        rows.map(|row| {
            let columns = row.map_err(|error| outbox_sqlite_error("read pending outbox", error))?;
            decode_outbox_columns(columns)
        })
        .collect()
    }

    fn mark_delivering(&self, id: &OutboxId) -> Result<OutboxRecord, PersistenceError> {
        let mut connection =
            self.connection
                .lock()
                .map_err(|_| PersistenceError::StorageIoFailure {
                    operation: "lock".to_owned(),
                    message: "storage connection mutex is poisoned".to_owned(),
                })?;
        let transaction = transaction_for_persistence(&mut connection, "begin outbox delivery")?;
        let columns = read_outbox_columns(&transaction, id)
            .map_err(|error| outbox_sqlite_error("read outbox delivery state", error))?
            .ok_or_else(|| PersistenceError::OutboxDeliveryFailed {
                message: format!("outbox '{id}' does not exist"),
            })?;
        let current = decode_outbox_columns(columns)?;
        if matches!(
            current.status(),
            OutboxStatus::Delivered | OutboxStatus::Failed
        ) {
            return Err(PersistenceError::OutboxDeliveryFailed {
                message: format!(
                    "outbox '{id}' cannot enter Delivering from {}",
                    current.status()
                ),
            });
        }
        let attempt_count = current.attempt_count().checked_add(1).ok_or_else(|| {
            PersistenceError::OutboxDeliveryFailed {
                message: format!("outbox '{id}' attempt counter exhausted"),
            }
        })?;
        transaction
            .execute(
                "UPDATE outbox
                 SET record_status = 'Delivering', attempt_count = ?1
                 WHERE outbox_id = ?2",
                params![
                    integer_persistence(attempt_count as u64, "attempt count")?,
                    id.as_str()
                ],
            )
            .map_err(|error| outbox_sqlite_error("mark outbox delivering", error))?;
        let updated = read_outbox_columns(&transaction, id)
            .map_err(|error| outbox_sqlite_error("read marked outbox", error))?
            .ok_or_else(|| PersistenceError::OutboxRecordCorrupt {
                message: format!("outbox '{id}' disappeared during delivery transition"),
            })?;
        let updated = decode_outbox_columns(updated)?;
        transaction
            .commit()
            .map_err(|error| outbox_sqlite_error("commit outbox delivery transition", error))?;
        Ok(updated)
    }

    fn mark_delivered(&self, id: &OutboxId) -> Result<(), PersistenceError> {
        let mut connection =
            self.connection
                .lock()
                .map_err(|_| PersistenceError::StorageIoFailure {
                    operation: "lock".to_owned(),
                    message: "storage connection mutex is poisoned".to_owned(),
                })?;
        let transaction = transaction_for_persistence(&mut connection, "begin outbox delivered")?;
        let columns = read_outbox_columns(&transaction, id)
            .map_err(|error| outbox_sqlite_error("read outbox delivery state", error))?
            .ok_or_else(|| PersistenceError::OutboxDeliveryFailed {
                message: format!("outbox '{id}' does not exist"),
            })?;
        let current = decode_outbox_columns(columns)?;
        match current.status() {
            OutboxStatus::Delivered => return Ok(()),
            OutboxStatus::Failed => {
                return Err(PersistenceError::OutboxDeliveryFailed {
                    message: format!("outbox '{id}' is already Failed"),
                });
            }
            OutboxStatus::Pending | OutboxStatus::Delivering => {}
        }
        transaction
            .execute(
                "UPDATE outbox SET record_status = 'Delivered' WHERE outbox_id = ?1",
                params![id.as_str()],
            )
            .map_err(|error| outbox_sqlite_error("mark outbox delivered", error))?;
        transaction
            .commit()
            .map_err(|error| outbox_sqlite_error("commit outbox delivered", error))
    }

    fn mark_failed(&self, id: &OutboxId, message: &str) -> Result<(), PersistenceError> {
        let mut connection =
            self.connection
                .lock()
                .map_err(|_| PersistenceError::StorageIoFailure {
                    operation: "lock".to_owned(),
                    message: "storage connection mutex is poisoned".to_owned(),
                })?;
        let transaction = transaction_for_persistence(&mut connection, "begin outbox failure")?;
        let columns = read_outbox_columns(&transaction, id)
            .map_err(|error| outbox_sqlite_error("read outbox failure state", error))?
            .ok_or_else(|| PersistenceError::OutboxDeliveryFailed {
                message: format!("outbox '{id}' does not exist"),
            })?;
        let current = decode_outbox_columns(columns)?;
        match current.status() {
            OutboxStatus::Failed => return Ok(()),
            OutboxStatus::Delivered => {
                return Err(PersistenceError::OutboxDeliveryFailed {
                    message: format!("outbox '{id}' is already Delivered"),
                });
            }
            OutboxStatus::Pending | OutboxStatus::Delivering => {}
        }
        let bounded_message = message.chars().take(4096).collect::<String>();
        transaction
            .execute(
                "UPDATE outbox SET record_status = 'Failed', failure_message = ?1
                 WHERE outbox_id = ?2",
                params![bounded_message, id.as_str()],
            )
            .map_err(|error| outbox_sqlite_error("mark outbox failed", error))?;
        transaction
            .commit()
            .map_err(|error| outbox_sqlite_error("commit outbox failure", error))
    }
}

impl AuditStore for SqliteStateBackend {
    fn append(&self, record: AuditRecord) -> Result<(), PersistenceError> {
        validate_audit_record(&record)?;
        let metadata = encode_metadata(record.metadata())?;
        let (runtime_incarnation, runtime_sequence) = optional_runtime_columns(record.runtime())?;
        let (causation_kind, causation_id) = causation_columns(record.causation());
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| audit_store_error("lock", "storage connection mutex is poisoned"))?;
        let transaction = transaction_for_audit(&mut connection, "begin audit append")?;
        transaction
            .execute(
                "INSERT INTO audit_records
                 (audit_id, record_type, principal, installation_id,
                  runtime_incarnation, runtime_sequence, correlation_id,
                  causation_kind, causation_id, outcome, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    integer_persistence(record.audit_id(), "audit id")?,
                    record.record_type(),
                    record.principal().map(PrincipalId::as_str),
                    record.installation().map(InstallationId::as_str),
                    runtime_incarnation,
                    runtime_sequence,
                    record.correlation().map(CorrelationId::as_str),
                    causation_kind,
                    causation_id,
                    record.outcome().to_string(),
                    metadata,
                ],
            )
            .map_err(|error| audit_store_sqlite_error("append audit record", error))?;
        transaction
            .commit()
            .map_err(|error| audit_store_sqlite_error("commit audit record", error))
    }

    fn list(&self, limit: usize) -> Result<Vec<AuditRecord>, PersistenceError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).map_err(|_| PersistenceError::StorageIoFailure {
            operation: "audit list".to_owned(),
            message: "audit listing limit exceeds SQLite integer range".to_owned(),
        })?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| audit_store_error("lock", "storage connection mutex is poisoned"))?;
        let mut statement = connection
            .prepare(
                "SELECT audit_id, record_type, principal, installation_id,
                        runtime_incarnation, runtime_sequence, correlation_id,
                        causation_kind, causation_id, outcome, metadata
                 FROM audit_records ORDER BY audit_id LIMIT ?1",
            )
            .map_err(|error| audit_store_sqlite_error("prepare audit listing", error))?;
        let rows = statement
            .query_map(params![limit], audit_columns)
            .map_err(|error| audit_store_sqlite_error("list audit records", error))?;
        rows.map(|row| {
            let columns =
                row.map_err(|error| audit_store_sqlite_error("read audit record", error))?;
            decode_audit_columns(columns)
        })
        .collect()
    }
}

impl JobStore for SqliteStateBackend {
    fn create(&self, job: JobRecord) -> Result<(), PersistenceError> {
        validate_job_record(&job)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| job_store_error("lock", "storage connection mutex is poisoned"))?;
        let transaction = transaction_for_job(&mut connection, "begin job create")?;
        insert_job(&transaction, &job)?;
        let result = transaction
            .commit()
            .map_err(|error| job_store_sqlite_error("commit job create", error));
        if result.is_ok() && job.state() == JobState::Running {
            crate::failpoints::hit("after-job-running-before-completion");
        }
        result
    }

    fn get(&self, id: &JobId) -> Result<Option<JobRecord>, PersistenceError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| job_store_error("lock", "storage connection mutex is poisoned"))?;
        let columns = connection
            .query_row(
                "SELECT job_id, owner, installation_id, state, deadline_millis,
                        wakeup_millis, cancellation_requested, attempt,
                        resource_budget, recovery_policy, correlation_id,
                        causation_kind, causation_id
                 FROM jobs WHERE job_id = ?1",
                params![id.as_str()],
                job_columns,
            )
            .optional()
            .map_err(|error| job_store_sqlite_error("read job", error))?;
        columns.map(decode_job_columns).transpose()
    }

    fn list(&self) -> Result<Vec<JobRecord>, PersistenceError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| job_store_error("lock", "storage connection mutex is poisoned"))?;
        let mut statement = connection
            .prepare(
                "SELECT job_id, owner, installation_id, state, deadline_millis,
                        wakeup_millis, cancellation_requested, attempt,
                        resource_budget, recovery_policy, correlation_id,
                        causation_kind, causation_id
                 FROM jobs ORDER BY job_id",
            )
            .map_err(|error| job_store_sqlite_error("prepare job listing", error))?;
        let rows = statement
            .query_map([], job_columns)
            .map_err(|error| job_store_sqlite_error("list jobs", error))?;
        rows.map(|row| {
            let columns = row.map_err(|error| job_store_sqlite_error("read job listing", error))?;
            decode_job_columns(columns)
        })
        .collect()
    }

    fn update(&self, job: JobRecord) -> Result<(), PersistenceError> {
        validate_job_record(&job)?;
        let (causation_kind, causation_id) = causation_columns(job.causation());
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| job_store_error("lock", "storage connection mutex is poisoned"))?;
        let transaction = transaction_for_job(&mut connection, "begin job update")?;
        let updated = transaction
            .execute(
                "UPDATE jobs SET owner = ?1, installation_id = ?2, state = ?3,
                        deadline_millis = ?4, wakeup_millis = ?5,
                        cancellation_requested = ?6, attempt = ?7,
                        resource_budget = ?8, recovery_policy = ?9,
                        correlation_id = ?10, causation_kind = ?11,
                        causation_id = ?12
                 WHERE job_id = ?13",
                params![
                    job.owner().as_str(),
                    job.installation().map(InstallationId::as_str),
                    job.state().to_string(),
                    optional_integer(job.deadline_millis(), "job deadline")?,
                    optional_integer(job.wakeup_millis(), "job wakeup")?,
                    if job.cancellation_requested() {
                        1_i64
                    } else {
                        0_i64
                    },
                    integer_persistence(job.attempt() as u64, "job attempt")?,
                    optional_integer(job.resource_budget(), "job resource budget")?,
                    job.recovery_policy().to_string(),
                    job.correlation().map(CorrelationId::as_str),
                    causation_kind,
                    causation_id,
                    job.job_id().as_str(),
                ],
            )
            .map_err(|error| job_store_sqlite_error("update job", error))?;
        if updated != 1 {
            return Err(PersistenceError::JobStoreFailure {
                message: format!("job '{}' does not exist", job.job_id()),
            });
        }
        transaction
            .commit()
            .map_err(|error| job_store_sqlite_error("commit job update", error))
    }

    fn recover_running(&self) -> Result<Vec<JobRecord>, PersistenceError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| job_store_error("lock", "storage connection mutex is poisoned"))?;
        let transaction = transaction_for_job(&mut connection, "begin job recovery")?;
        let mut statement = transaction
            .prepare(
                "SELECT job_id, owner, installation_id, state, deadline_millis,
                        wakeup_millis, cancellation_requested, attempt,
                        resource_budget, recovery_policy, correlation_id,
                        causation_kind, causation_id
                 FROM jobs WHERE state = 'Running' ORDER BY job_id",
            )
            .map_err(|error| job_store_sqlite_error("prepare running job recovery", error))?;
        let rows = statement
            .query_map([], job_columns)
            .map_err(|error| job_store_sqlite_error("list running jobs", error))?;
        let running = rows
            .map(|row| {
                let columns =
                    row.map_err(|error| job_store_sqlite_error("read running job", error))?;
                decode_job_columns(columns)
            })
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        transaction
            .execute(
                "UPDATE jobs SET state = 'Interrupted' WHERE state = 'Running'",
                [],
            )
            .map_err(|error| job_store_sqlite_error("mark running jobs interrupted", error))?;
        transaction
            .commit()
            .map_err(|error| job_store_sqlite_error("commit job recovery", error))?;
        Ok(running
            .into_iter()
            .map(|job| job.with_state(JobState::Interrupted))
            .collect())
    }
}

/// SQLite-backed implementation of the kernel's durable event journal.
///
/// Journal records are separate from installation state and are never read by
/// the StateBackend to reconstruct application state.
pub struct SqliteEventJournal {
    profile_root: PathBuf,
    connection: Mutex<Connection>,
}

impl SqliteEventJournal {
    pub fn open(profile_root: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let profile_root = canonical_profile_root(profile_root.as_ref())?;
        let connection = open_profile_connection(&profile_root)?;
        Ok(Self {
            profile_root,
            connection: Mutex::new(connection),
        })
    }

    pub fn profile_root(&self) -> &Path {
        &self.profile_root
    }

    pub fn database_path(&self) -> PathBuf {
        self.profile_root.join(DATABASE_FILE)
    }
}

impl EventJournal for SqliteEventJournal {
    fn append(&self, event: &EventEnvelope) -> Result<(), EventJournalError> {
        if event.delivery_mode() != DeliveryMode::Durable {
            return Err(journal_append_error(
                "only Durable events may be appended to the production journal",
            ));
        }
        let encoded = encode_event(event).map_err(|message| journal_append_error(&message))?;
        let digest: [u8; 32] = Sha256::digest(&encoded).into();
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| journal_append_error("journal connection mutex is poisoned"))?;
        let transaction = transaction_for_journal(&mut connection, "begin journal append")?;
        let existing: Option<(Vec<u8>, Vec<u8>)> = transaction
            .query_row(
                "SELECT record, record_hash FROM event_journal WHERE event_id = ?1",
                params![event.event_id().as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| journal_append_sqlite_error("read existing journal event", error))?;
        if let Some((existing_record, existing_hash)) = existing {
            if existing_record == encoded && existing_hash == digest.as_slice() {
                transaction.commit().map_err(|error| {
                    journal_append_sqlite_error("commit idempotent journal append", error)
                })?;
                return Ok(());
            }
            return Err(journal_append_error(&format!(
                "JournalAppendFailed: event '{}' already exists with different bytes",
                event.event_id()
            )));
        }
        transaction
            .execute(
                "INSERT INTO event_journal(event_id, record, record_hash)
                 VALUES (?1, ?2, ?3)",
                params![event.event_id().as_str(), encoded, digest.as_slice()],
            )
            .map_err(|error| journal_append_sqlite_error("insert journal event", error))?;
        transaction
            .commit()
            .map_err(|error| journal_append_sqlite_error("commit journal append", error))
    }

    fn read_from(&self, cursor: EventCursor) -> Result<Vec<EventEnvelope>, EventJournalError> {
        let cursor = i64::try_from(cursor.position()).map_err(|_| {
            journal_append_error("JournalCursorInvalid: cursor exceeds SQLite integer range")
        })?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| journal_replay_error("journal connection mutex is poisoned"))?;
        let mut statement = connection
            .prepare(
                "SELECT sequence, event_id, record, record_hash
                 FROM event_journal WHERE sequence > ?1 ORDER BY sequence",
            )
            .map_err(|error| journal_replay_sqlite_error("prepare journal replay", error))?;
        let rows = statement
            .query_map(params![cursor], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })
            .map_err(|error| journal_replay_sqlite_error("read journal replay", error))?;
        let mut events = Vec::new();
        for row in rows {
            let (sequence, event_id, record, record_hash) =
                row.map_err(|error| journal_replay_sqlite_error("decode journal row", error))?;
            if sequence <= 0 {
                return Err(journal_replay_error(
                    "JournalReplayFailed: invalid journal sequence",
                ));
            }
            let expected_hash: [u8; 32] = Sha256::digest(&record).into();
            if record_hash.as_slice() != expected_hash.as_slice() {
                return Err(journal_replay_error(&format!(
                    "JournalReplayFailed: hash mismatch at sequence {sequence}"
                )));
            }
            let event = decode_event(&record).map_err(|message| {
                journal_replay_error(&format!("JournalReplayFailed: {message}"))
            })?;
            if event.event_id().as_str() != event_id
                || event.delivery_mode() != DeliveryMode::Durable
            {
                return Err(journal_replay_error(&format!(
                    "JournalReplayFailed: envelope identity or delivery mode mismatch at sequence {sequence}"
                )));
            }
            events.push(event);
        }
        Ok(events)
    }
}

fn validate_outbox_for_installation(
    installation: &InstallationId,
    outbox: &OutboxRecord,
) -> Result<(), StateError> {
    if outbox.installation() != installation {
        return Err(StateError::Persistence(
            PersistenceError::OutboxRecordCorrupt {
                message: format!(
                    "outbox '{}' belongs to installation '{}', expected '{}'",
                    outbox.outbox_id(),
                    outbox.installation(),
                    installation
                ),
            },
        ));
    }
    if outbox.outbox_id().as_str().trim().is_empty() {
        return Err(StateError::Persistence(
            PersistenceError::OutboxRecordCorrupt {
                message: "outbox identity must not be empty".to_owned(),
            },
        ));
    }
    if outbox.status() != OutboxStatus::Pending {
        return Err(StateError::Persistence(
            PersistenceError::OutboxAppendFailed {
                message: format!(
                    "outbox '{}' must enter storage as Pending, got {}",
                    outbox.outbox_id(),
                    outbox.status()
                ),
            },
        ));
    }
    if outbox.event().delivery_mode() != DeliveryMode::Durable {
        return Err(StateError::Persistence(
            PersistenceError::OutboxAppendFailed {
                message: format!(
                    "outbox '{}' contains a non-durable event",
                    outbox.outbox_id()
                ),
            },
        ));
    }
    Ok(())
}

fn state_error_to_persistence(error: StateError) -> PersistenceError {
    match error {
        StateError::Persistence(error) => error,
        other => PersistenceError::OutboxAppendFailed {
            message: other.to_string(),
        },
    }
}

fn transaction_for_persistence<'a>(
    connection: &'a mut Connection,
    operation: &'static str,
) -> Result<Transaction<'a>, PersistenceError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| outbox_sqlite_error(operation, error))
}

fn transaction_for_journal<'a>(
    connection: &'a mut Connection,
    operation: &'static str,
) -> Result<Transaction<'a>, EventJournalError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| journal_append_sqlite_error(operation, error))
}

fn insert_outbox(
    transaction: &Transaction<'_>,
    record: &OutboxRecord,
    encoded_event: &[u8],
) -> Result<(), StateError> {
    transaction
        .execute(
            "INSERT INTO outbox
             (outbox_id, installation_id, event_id, record_status, attempt_count,
              created_sequence, created_at_millis, record, failure_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
            params![
                record.outbox_id().as_str(),
                record.installation().as_str(),
                record.event().event_id().as_str(),
                record.status().to_string(),
                integer(record.attempt_count() as u64, "outbox attempt count")?,
                integer(record.created_sequence(), "outbox sequence")?,
                integer(record.created_at_millis(), "outbox timestamp")?,
                encoded_event,
            ],
        )
        .map_err(|error| {
            StateError::Persistence(PersistenceError::OutboxAppendFailed {
                message: format!("insert outbox record: {error}"),
            })
        })?;
    Ok(())
}

fn read_outbox_columns(
    connection: &Connection,
    id: &OutboxId,
) -> rusqlite::Result<Option<OutboxColumns>> {
    connection
        .query_row(
            "SELECT outbox_id, installation_id, event_id, record_status,
                    attempt_count, created_sequence, created_at_millis, record
             FROM outbox WHERE outbox_id = ?1",
            params![id.as_str()],
            outbox_columns,
        )
        .optional()
}

fn outbox_columns(row: &Row<'_>) -> rusqlite::Result<OutboxColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn decode_outbox_columns(columns: OutboxColumns) -> Result<OutboxRecord, PersistenceError> {
    let (
        outbox_id,
        installation,
        event_id,
        status,
        attempt_count,
        created_sequence,
        created_at_millis,
        encoded_event,
    ) = columns;
    let event =
        decode_event(&encoded_event).map_err(|message| PersistenceError::OutboxRecordCorrupt {
            message: format!("outbox '{outbox_id}' event: {message}"),
        })?;
    if event.event_id().as_str() != event_id {
        return Err(PersistenceError::OutboxRecordCorrupt {
            message: format!(
                "outbox '{outbox_id}' column event id '{event_id}' does not match envelope '{}'",
                event.event_id()
            ),
        });
    }
    if event.delivery_mode() != DeliveryMode::Durable {
        return Err(PersistenceError::OutboxRecordCorrupt {
            message: format!("outbox '{outbox_id}' contains a non-durable event"),
        });
    }
    let status = outbox_status_from_string(&status)?;
    let attempt_count = u32::try_from(unsigned_persistence(attempt_count, "outbox attempt count")?)
        .map_err(|_| PersistenceError::OutboxRecordCorrupt {
            message: format!("outbox '{outbox_id}' attempt count exceeds u32"),
        })?;
    Ok(OutboxRecord::new(
        OutboxId::new(outbox_id),
        InstallationId::new(installation),
        event,
        unsigned_persistence(created_sequence, "outbox sequence")?,
        unsigned_persistence(created_at_millis, "outbox timestamp")?,
    )
    .with_status(status, attempt_count))
}

fn outbox_status_from_string(value: &str) -> Result<OutboxStatus, PersistenceError> {
    match value {
        "Pending" => Ok(OutboxStatus::Pending),
        "Delivering" => Ok(OutboxStatus::Delivering),
        "Delivered" => Ok(OutboxStatus::Delivered),
        "Failed" => Ok(OutboxStatus::Failed),
        other => Err(PersistenceError::OutboxRecordCorrupt {
            message: format!("unknown outbox status '{other}'"),
        }),
    }
}

fn integer_persistence(value: u64, name: &str) -> Result<i64, PersistenceError> {
    i64::try_from(value).map_err(|_| PersistenceError::OutboxDeliveryFailed {
        message: format!("{name} exceeds SQLite integer range"),
    })
}

fn unsigned_persistence(value: i64, name: &str) -> Result<u64, PersistenceError> {
    u64::try_from(value).map_err(|_| PersistenceError::OutboxRecordCorrupt {
        message: format!("{name} contains negative value {value}"),
    })
}

fn validate_audit_record(record: &AuditRecord) -> Result<(), PersistenceError> {
    if record.record_type().trim().is_empty() {
        return Err(PersistenceError::InvalidRecord {
            message: "audit record type must not be empty".to_owned(),
        });
    }
    if record.record_type().len() > MAX_CODEC_STRING_BYTES {
        return Err(PersistenceError::InvalidRecord {
            message: "audit record type exceeds the storage record limit".to_owned(),
        });
    }
    for (key, value) in record.metadata() {
        let normalized = key.to_ascii_lowercase();
        if [
            "payload",
            "credential",
            "password",
            "secret",
            "token",
            "raw_state",
            "state_value",
        ]
        .iter()
        .any(|forbidden| normalized.contains(forbidden))
        {
            return Err(PersistenceError::InvalidRecord {
                message: format!("audit metadata key '{key}' is not allowed"),
            });
        }
        if key.len() > MAX_CODEC_STRING_BYTES || value.len() > MAX_CODEC_STRING_BYTES {
            return Err(PersistenceError::InvalidRecord {
                message: "audit metadata exceeds the storage record limit".to_owned(),
            });
        }
    }
    if record.correlation().is_none() && record.causation().is_some() {
        return Err(PersistenceError::InvalidRecord {
            message: "audit causation requires a correlation id".to_owned(),
        });
    }
    Ok(())
}

fn encode_metadata(
    metadata: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<u8>, PersistenceError> {
    if metadata.len() > u32::MAX as usize {
        return Err(PersistenceError::InvalidRecord {
            message: "audit metadata has too many entries".to_owned(),
        });
    }
    let mut encoder = Encoder::new();
    encoder.u32(metadata.len() as u32);
    for (key, value) in metadata {
        encoder
            .string(key, "audit metadata key")
            .map_err(|message| PersistenceError::InvalidRecord { message })?;
        encoder
            .string(value, "audit metadata value")
            .map_err(|message| PersistenceError::InvalidRecord { message })?;
    }
    Ok(encoder.finish())
}

fn decode_metadata(
    bytes: &[u8],
) -> Result<std::collections::BTreeMap<String, String>, PersistenceError> {
    let mut decoder = Decoder::new(bytes);
    let count = decoder
        .u32()
        .map_err(|message| PersistenceError::StorageCorrupt { message })?;
    if count > 10_000 {
        return Err(PersistenceError::StorageCorrupt {
            message: "audit metadata entry count exceeds the safety limit".to_owned(),
        });
    }
    let mut metadata = std::collections::BTreeMap::new();
    for _ in 0..count {
        let key = decoder
            .string("audit metadata key")
            .map_err(|message| PersistenceError::StorageCorrupt { message })?;
        let value = decoder
            .string("audit metadata value")
            .map_err(|message| PersistenceError::StorageCorrupt { message })?;
        if metadata.insert(key.clone(), value).is_some() {
            return Err(PersistenceError::StorageCorrupt {
                message: format!("duplicate audit metadata key '{key}'"),
            });
        }
    }
    decoder
        .finish()
        .map_err(|message| PersistenceError::StorageCorrupt { message })?;
    Ok(metadata)
}

fn optional_runtime_columns(
    runtime: Option<RuntimeId>,
) -> Result<(Option<i64>, Option<i64>), PersistenceError> {
    runtime
        .map(|runtime| {
            Ok((
                Some(integer_persistence(
                    runtime.incarnation(),
                    "runtime incarnation",
                )?),
                Some(integer_persistence(runtime.sequence(), "runtime sequence")?),
            ))
        })
        .transpose()
        .map(|value| value.unwrap_or((None, None)))
}

fn causation_columns(causation: Option<&CausationRef>) -> (Option<&str>, Option<&str>) {
    match causation {
        None => (None, None),
        Some(CausationRef::Invocation(invocation)) => {
            (Some("Invocation"), Some(invocation.as_str()))
        }
        Some(CausationRef::Event(event)) => (Some("Event"), Some(event.as_str())),
    }
}

fn decode_causation_columns(
    kind: Option<String>,
    id: Option<String>,
) -> Result<Option<CausationRef>, PersistenceError> {
    match (kind, id) {
        (None, None) => Ok(None),
        (Some(kind), Some(id)) => match kind.as_str() {
            "Invocation" => Ok(Some(CausationRef::Invocation(InvocationId::new(id)))),
            "Event" => Ok(Some(CausationRef::Event(EventId::new(id)))),
            other => Err(PersistenceError::StorageCorrupt {
                message: format!("unknown causation kind '{other}'"),
            }),
        },
        _ => Err(PersistenceError::StorageCorrupt {
            message: "causation kind and identity must be present together".to_owned(),
        }),
    }
}

fn audit_columns(row: &Row<'_>) -> rusqlite::Result<AuditColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
    ))
}

fn decode_audit_columns(columns: AuditColumns) -> Result<AuditRecord, PersistenceError> {
    let (
        audit_id,
        record_type,
        principal,
        installation,
        runtime_incarnation,
        runtime_sequence,
        correlation,
        causation_kind,
        causation_id,
        outcome,
        metadata,
    ) = columns;
    let audit_id = unsigned_persistence(audit_id, "audit id")?;
    let runtime = match (runtime_incarnation, runtime_sequence) {
        (None, None) => None,
        (Some(incarnation), Some(sequence)) => Some((
            unsigned_persistence(incarnation, "runtime incarnation")?,
            unsigned_persistence(sequence, "runtime sequence")?,
        )),
        _ => {
            return Err(PersistenceError::StorageCorrupt {
                message: "runtime incarnation and sequence must be present together".to_owned(),
            });
        }
    };
    let causation = decode_causation_columns(causation_kind, causation_id)?;
    let outcome = audit_outcome_from_string(&outcome)?;
    let metadata = decode_metadata(&metadata)?;
    let mut record = AuditRecord::new(audit_id, record_type, outcome, metadata);
    if let Some(principal) = principal {
        record = record.with_principal(PrincipalId::new(principal));
    }
    if let Some(installation) = installation {
        record = record.with_installation(InstallationId::new(installation));
    }
    if let Some((incarnation, sequence)) = runtime {
        record = record.with_runtime_parts(incarnation, sequence);
    }
    match (correlation, causation) {
        (None, None) => {}
        (Some(correlation), causation) => {
            record = record.with_trace(CorrelationId::new(correlation), causation);
        }
        (None, Some(_)) => {
            return Err(PersistenceError::StorageCorrupt {
                message: "audit causation exists without correlation".to_owned(),
            });
        }
    }
    Ok(record)
}

fn audit_outcome_from_string(value: &str) -> Result<AuditOutcome, PersistenceError> {
    match value {
        "Committed" => Ok(AuditOutcome::Committed),
        "NotCommitted" => Ok(AuditOutcome::NotCommitted),
        "PendingDelivery" => Ok(AuditOutcome::PendingDelivery),
        "Interrupted" => Ok(AuditOutcome::Interrupted),
        "Failed" => Ok(AuditOutcome::Failed),
        other => Err(PersistenceError::StorageCorrupt {
            message: format!("unknown audit outcome '{other}'"),
        }),
    }
}

fn audit_store_error(operation: &str, message: &str) -> PersistenceError {
    PersistenceError::StorageIoFailure {
        operation: format!("audit {operation}"),
        message: message.to_owned(),
    }
}

fn transaction_for_audit<'a>(
    connection: &'a mut Connection,
    operation: &'static str,
) -> Result<Transaction<'a>, PersistenceError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| audit_store_sqlite_error(operation, error))
}

fn audit_store_sqlite_error(operation: &str, error: rusqlite::Error) -> PersistenceError {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                PersistenceError::StorageBusy {
                    message: format!("audit {operation}: database is busy"),
                }
            }
            _ => audit_store_error(operation, &failure.to_string()),
        },
        rusqlite::Error::FromSqlConversionFailure(_, Type::Blob, source) => {
            PersistenceError::StorageCorrupt {
                message: format!("audit {operation}: {source}"),
            }
        }
        other => audit_store_error(operation, &other.to_string()),
    }
}

fn validate_job_record(job: &JobRecord) -> Result<(), PersistenceError> {
    if job.job_id().as_str().trim().is_empty() {
        return Err(PersistenceError::InvalidRecord {
            message: "job identity must not be empty".to_owned(),
        });
    }
    if !job.owner().is_well_formed() {
        return Err(PersistenceError::InvalidRecord {
            message: "job owner must be well formed".to_owned(),
        });
    }
    if let Some(installation) = job.installation()
        && installation.as_str().trim().is_empty()
    {
        return Err(PersistenceError::InvalidRecord {
            message: "job installation identity must not be empty".to_owned(),
        });
    }
    if job.correlation().is_none() && job.causation().is_some() {
        return Err(PersistenceError::InvalidRecord {
            message: "job causation requires a correlation id".to_owned(),
        });
    }
    Ok(())
}

fn insert_job(transaction: &Transaction<'_>, job: &JobRecord) -> Result<(), PersistenceError> {
    let (causation_kind, causation_id) = causation_columns(job.causation());
    transaction
        .execute(
            "INSERT INTO jobs
             (job_id, owner, installation_id, state, deadline_millis, wakeup_millis,
              cancellation_requested, attempt, resource_budget, recovery_policy,
              correlation_id, causation_kind, causation_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                job.job_id().as_str(),
                job.owner().as_str(),
                job.installation().map(InstallationId::as_str),
                job.state().to_string(),
                optional_integer(job.deadline_millis(), "job deadline")?,
                optional_integer(job.wakeup_millis(), "job wakeup")?,
                if job.cancellation_requested() {
                    1_i64
                } else {
                    0_i64
                },
                integer_persistence(job.attempt() as u64, "job attempt")?,
                optional_integer(job.resource_budget(), "job resource budget")?,
                job.recovery_policy().to_string(),
                job.correlation().map(CorrelationId::as_str),
                causation_kind,
                causation_id,
            ],
        )
        .map_err(|error| job_store_sqlite_error("insert job", error))?;
    Ok(())
}

fn job_columns(row: &Row<'_>) -> rusqlite::Result<JobColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
    ))
}

fn decode_job_columns(columns: JobColumns) -> Result<JobRecord, PersistenceError> {
    let (
        job_id,
        owner,
        installation,
        state,
        deadline_millis,
        wakeup_millis,
        cancellation_requested,
        attempt,
        resource_budget,
        recovery_policy,
        correlation,
        causation_kind,
        causation_id,
    ) = columns;
    let cancellation_requested = match cancellation_requested {
        0 => false,
        1 => true,
        other => {
            return Err(PersistenceError::StorageCorrupt {
                message: format!("job cancellation flag must be 0 or 1, got {other}"),
            });
        }
    };
    let attempt = u32::try_from(unsigned_persistence(attempt, "job attempt")?).map_err(|_| {
        PersistenceError::StorageCorrupt {
            message: "job attempt exceeds u32".to_owned(),
        }
    })?;
    let causation = decode_causation_columns(causation_kind, causation_id)?;
    let mut job = JobRecord::new(JobId::new(job_id), PrincipalId::new(owner))
        .with_state(job_state_from_string(&state)?)
        .with_deadline_millis(optional_unsigned(deadline_millis, "job deadline")?)
        .with_wakeup_millis(optional_unsigned(wakeup_millis, "job wakeup")?)
        .with_cancellation_requested(cancellation_requested)
        .with_attempt(attempt)
        .with_resource_budget(optional_unsigned(resource_budget, "job resource budget")?)
        .with_recovery_policy(job_recovery_policy_from_string(&recovery_policy)?);
    if let Some(installation) = installation {
        job = job.with_installation(InstallationId::new(installation));
    }
    match (correlation, causation) {
        (None, None) => {}
        (Some(correlation), causation) => {
            job = job.with_trace(CorrelationId::new(correlation), causation);
        }
        (None, Some(_)) => {
            return Err(PersistenceError::StorageCorrupt {
                message: "job causation exists without correlation".to_owned(),
            });
        }
    }
    Ok(job)
}

fn optional_integer(value: Option<u64>, name: &str) -> Result<Option<i64>, PersistenceError> {
    value
        .map(|value| integer_persistence(value, name))
        .transpose()
}

fn optional_unsigned(value: Option<i64>, name: &str) -> Result<Option<u64>, PersistenceError> {
    value
        .map(|value| unsigned_persistence(value, name))
        .transpose()
}

fn job_state_from_string(value: &str) -> Result<JobState, PersistenceError> {
    match value {
        "Pending" => Ok(JobState::Pending),
        "Waiting" => Ok(JobState::Waiting),
        "Runnable" => Ok(JobState::Runnable),
        "Running" => Ok(JobState::Running),
        "Completed" => Ok(JobState::Completed),
        "Cancelled" => Ok(JobState::Cancelled),
        "Failed" => Ok(JobState::Failed),
        "Interrupted" => Ok(JobState::Interrupted),
        other => Err(PersistenceError::StorageCorrupt {
            message: format!("unknown job state '{other}'"),
        }),
    }
}

fn job_recovery_policy_from_string(value: &str) -> Result<JobRecoveryPolicy, PersistenceError> {
    match value {
        "Manual" => Ok(JobRecoveryPolicy::Manual),
        "RetrySafe" => Ok(JobRecoveryPolicy::RetrySafe),
        "Idempotent" => Ok(JobRecoveryPolicy::Idempotent),
        other => Err(PersistenceError::StorageCorrupt {
            message: format!("unknown job recovery policy '{other}'"),
        }),
    }
}

fn job_store_error(operation: &str, message: &str) -> PersistenceError {
    PersistenceError::JobStoreFailure {
        message: format!("{operation}: {message}"),
    }
}

fn transaction_for_job<'a>(
    connection: &'a mut Connection,
    operation: &'static str,
) -> Result<Transaction<'a>, PersistenceError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| job_store_sqlite_error(operation, error))
}

fn job_store_sqlite_error(operation: &str, error: rusqlite::Error) -> PersistenceError {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                PersistenceError::StorageBusy {
                    message: format!("job {operation}: database is busy"),
                }
            }
            _ => job_store_error(operation, &failure.to_string()),
        },
        rusqlite::Error::FromSqlConversionFailure(_, Type::Integer, source) => {
            PersistenceError::StorageCorrupt {
                message: format!("job {operation}: {source}"),
            }
        }
        other => job_store_error(operation, &other.to_string()),
    }
}

fn outbox_sqlite_error(operation: &str, error: rusqlite::Error) -> PersistenceError {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                PersistenceError::StorageBusy {
                    message: format!("{operation}: database is busy"),
                }
            }
            _ => PersistenceError::OutboxAppendFailed {
                message: format!("{operation}: {failure}"),
            },
        },
        rusqlite::Error::FromSqlConversionFailure(_, Type::Blob, source) => {
            PersistenceError::OutboxRecordCorrupt {
                message: format!("{operation}: {source}"),
            }
        }
        other => PersistenceError::OutboxAppendFailed {
            message: format!("{operation}: {other}"),
        },
    }
}

fn journal_append_error(message: &str) -> EventJournalError {
    EventJournalError::Failure(format!("JournalAppendFailed: {message}"))
}

fn journal_replay_error(message: &str) -> EventJournalError {
    EventJournalError::Failure(message.to_owned())
}

fn journal_append_sqlite_error(operation: &str, error: rusqlite::Error) -> EventJournalError {
    journal_append_error(&format!("{operation}: {error}"))
}

fn journal_replay_sqlite_error(operation: &str, error: rusqlite::Error) -> EventJournalError {
    journal_replay_error(&format!("JournalReplayFailed: {operation}: {error}"))
}

fn encode_event(event: &EventEnvelope) -> Result<Vec<u8>, String> {
    let mut encoder = Encoder::new();
    encoder.raw(EVENT_CODEC_MAGIC);
    encoder.string(event.event_id().as_str(), "event id")?;
    encoder.string(event.contract().namespace(), "event namespace")?;
    encoder.string(event.contract().name(), "event name")?;
    encoder.u16(event.contract().version().major());
    encoder.u16(event.contract().version().minor());
    encoder.string(event.producer().as_str(), "event producer")?;
    encode_optional_runtime(&mut encoder, event.producer_runtime_id());
    encoder.u64(event.sequence());
    encoder.string(event.correlation_id().as_str(), "correlation id")?;
    encode_causation(&mut encoder, event.causation())?;
    encoder.u8(match event.delivery_mode() {
        DeliveryMode::Ephemeral => 0,
        DeliveryMode::Durable => 1,
    });
    encoder.bytes(event.payload(), MAX_CODEC_PAYLOAD_BYTES, "event payload")?;
    match event.invocation_completed() {
        Some(metadata) => {
            encoder.u8(1);
            encoder.string(metadata.request_id().as_str(), "completed request id")?;
            encoder.string(metadata.invocation_id().as_str(), "completed invocation id")?;
            encoder.string(metadata.caller().as_str(), "completed caller")?;
            encoder.u64(metadata.provider_runtime_id().incarnation());
            encoder.u64(metadata.provider_runtime_id().sequence());
            encoder.string(
                metadata.capability().namespace(),
                "completed capability namespace",
            )?;
            encoder.string(metadata.capability().name(), "completed capability name")?;
            encoder.u16(metadata.capability().interface_major());
            encoder.string(metadata.operation().as_str(), "completed operation")?;
            encoder.u8(encode_outcome(metadata.outcome()));
        }
        None => encoder.u8(0),
    }
    Ok(encoder.finish())
}

fn encode_optional_runtime(encoder: &mut Encoder, runtime: Option<RuntimeId>) {
    match runtime {
        Some(runtime) => {
            encoder.u8(1);
            encoder.u64(runtime.incarnation());
            encoder.u64(runtime.sequence());
        }
        None => encoder.u8(0),
    }
}

fn encode_causation(encoder: &mut Encoder, causation: Option<&CausationRef>) -> Result<(), String> {
    match causation {
        None => {
            encoder.u8(0);
            Ok(())
        }
        Some(CausationRef::Invocation(invocation)) => {
            encoder.u8(1);
            encoder.string(invocation.as_str(), "causation invocation")
        }
        Some(CausationRef::Event(event)) => {
            encoder.u8(2);
            encoder.string(event.as_str(), "causation event")
        }
    }
}

fn encode_outcome(outcome: RpcOutcomeClass) -> u8 {
    match outcome {
        RpcOutcomeClass::Success => 0,
        RpcOutcomeClass::AuthorizationDenied => 1,
        RpcOutcomeClass::NoCompatibleProvider => 2,
        RpcOutcomeClass::ProviderBusy => 3,
        RpcOutcomeClass::QueueFull => 4,
        RpcOutcomeClass::DeadlineExceeded => 5,
        RpcOutcomeClass::Cancelled => 6,
        RpcOutcomeClass::ProviderReturnedError => 7,
        RpcOutcomeClass::ProviderCrashed => 8,
        RpcOutcomeClass::RuntimeUnavailable => 9,
        RpcOutcomeClass::InvalidRetryClassification => 10,
        RpcOutcomeClass::InvalidIdempotencyKey => 11,
    }
}

fn decode_outcome(value: u8) -> Result<RpcOutcomeClass, String> {
    match value {
        0 => Ok(RpcOutcomeClass::Success),
        1 => Ok(RpcOutcomeClass::AuthorizationDenied),
        2 => Ok(RpcOutcomeClass::NoCompatibleProvider),
        3 => Ok(RpcOutcomeClass::ProviderBusy),
        4 => Ok(RpcOutcomeClass::QueueFull),
        5 => Ok(RpcOutcomeClass::DeadlineExceeded),
        6 => Ok(RpcOutcomeClass::Cancelled),
        7 => Ok(RpcOutcomeClass::ProviderReturnedError),
        8 => Ok(RpcOutcomeClass::ProviderCrashed),
        9 => Ok(RpcOutcomeClass::RuntimeUnavailable),
        10 => Ok(RpcOutcomeClass::InvalidRetryClassification),
        11 => Ok(RpcOutcomeClass::InvalidIdempotencyKey),
        other => Err(format!("unknown RPC outcome code {other}")),
    }
}

fn decode_event(bytes: &[u8]) -> Result<EventEnvelope, String> {
    let mut decoder = Decoder::new(bytes);
    decoder.raw(EVENT_CODEC_MAGIC)?;
    let event_id = EventId::new(decoder.string("event id")?);
    let contract = EventContract::new(
        decoder.string("event namespace")?,
        decoder.string("event name")?,
        InterfaceVersion::new(decoder.u16()?, decoder.u16()?),
    );
    let producer = PrincipalId::new(decoder.string("event producer")?);
    let producer_runtime_id = decode_optional_runtime(&mut decoder)?;
    let sequence = decoder.u64()?;
    let correlation_id = CorrelationId::new(decoder.string("correlation id")?);
    let causation = decode_causation(&mut decoder)?;
    let delivery_mode = match decoder.u8()? {
        0 => DeliveryMode::Ephemeral,
        1 => DeliveryMode::Durable,
        other => return Err(format!("unknown event delivery mode code {other}")),
    };
    let payload = decoder.bytes(MAX_CODEC_PAYLOAD_BYTES, "event payload")?;
    let invocation_completed = match decoder.u8()? {
        0 => None,
        1 => Some(InvocationCompletedMetadata::from_storage_parts(
            RpcRequestId::new(decoder.string("completed request id")?),
            InvocationId::new(decoder.string("completed invocation id")?),
            PrincipalId::new(decoder.string("completed caller")?),
            decoder.u64()?,
            decoder.u64()?,
            worldline_capability_contract(
                decoder.string("completed capability namespace")?,
                decoder.string("completed capability name")?,
                decoder.u16()?,
            )?,
            OperationId::new(decoder.string("completed operation")?),
            decode_outcome(decoder.u8()?)?,
        )),
        other => {
            return Err(format!(
                "invalid invocation-completed presence code {other}"
            ));
        }
    };
    decoder.finish()?;
    if event_id.as_str().trim().is_empty()
        || !contract.is_well_formed()
        || !producer.is_well_formed()
        || correlation_id.as_str().trim().is_empty()
    {
        return Err("event envelope contains an empty or malformed identity".to_owned());
    }
    Ok(EventEnvelope::from_storage_parts(
        event_id,
        contract,
        producer,
        producer_runtime_id,
        sequence,
        correlation_id,
        causation,
        delivery_mode,
        payload,
        invocation_completed,
    ))
}

fn worldline_capability_contract(
    namespace: String,
    name: String,
    interface_major: u16,
) -> Result<worldline_kernel::CapabilityContract, String> {
    let contract = worldline_kernel::CapabilityContract::new(namespace, name, interface_major);
    if contract.is_well_formed() {
        Ok(contract)
    } else {
        Err("completed capability contract is malformed".to_owned())
    }
}

fn decode_optional_runtime(decoder: &mut Decoder<'_>) -> Result<Option<(u64, u64)>, String> {
    match decoder.u8()? {
        0 => Ok(None),
        1 => Ok(Some((decoder.u64()?, decoder.u64()?))),
        other => Err(format!("invalid runtime presence code {other}")),
    }
}

fn decode_causation(decoder: &mut Decoder<'_>) -> Result<Option<CausationRef>, String> {
    match decoder.u8()? {
        0 => Ok(None),
        1 => Ok(Some(CausationRef::Invocation(InvocationId::new(
            decoder.string("causation invocation")?,
        )))),
        2 => Ok(Some(CausationRef::Event(EventId::new(
            decoder.string("causation event")?,
        )))),
        other => Err(format!("unknown causation code {other}")),
    }
}

struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn bytes(&mut self, bytes: &[u8], maximum: usize, label: &str) -> Result<(), String> {
        if bytes.len() > maximum || bytes.len() > u32::MAX as usize {
            return Err(format!("{label} exceeds the storage record limit"));
        }
        self.bytes
            .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn string(&mut self, value: &str, label: &str) -> Result<(), String> {
        self.bytes(value.as_bytes(), MAX_CODEC_STRING_BYTES, label)
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn raw(&mut self, expected: &[u8]) -> Result<(), String> {
        let actual = self.take(expected.len(), "record magic")?;
        if actual != expected {
            return Err("unsupported event record codec".to_owned());
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1, "u8")?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let bytes = self.take(2, "u16")?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let bytes = self.take(8, "u64")?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn bytes(&mut self, maximum: usize, label: &str) -> Result<Vec<u8>, String> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(format!("{label} exceeds the storage record limit"));
        }
        Ok(self.take(length, label)?.to_vec())
    }

    fn string(&mut self, label: &str) -> Result<String, String> {
        let bytes = self.bytes(MAX_CODEC_STRING_BYTES, label)?;
        String::from_utf8(bytes).map_err(|_| format!("{label} is not valid UTF-8"))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4, "u32")?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn take(&mut self, length: usize, label: &str) -> Result<&'a [u8], String> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| format!("{label} length overflows record bounds"))?;
        if end > self.bytes.len() {
            return Err(format!("{label} is truncated"));
        }
        let result = &self.bytes[self.position..end];
        self.position = end;
        Ok(result)
    }

    fn finish(self) -> Result<(), String> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err("event record has trailing bytes".to_owned())
        }
    }
}

fn insert_state(transaction: &Transaction<'_>, state: &BackendState) -> Result<(), StateError> {
    let record = state.record();
    transaction
        .execute(
            "INSERT INTO installations
             (installation_id, plugin_id, state_schema_version, status, revision, runtime_generation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.installation_id().as_str(),
                record.plugin_id().as_str(),
                integer(record.state_schema_version().value(), "schema version")?,
                record.status().to_string(),
                integer(record.state_revision().value(), "record revision")?,
                integer(record.runtime_generation(), "runtime generation")?,
            ],
        )
        .map_err(|error| map_sqlite_error("insert installation", error))?;
    for (key, value) in state.values() {
        transaction
            .execute(
                "INSERT INTO state_entries(installation_id, state_key, state_value)
                 VALUES (?1, ?2, ?3)",
                params![record.installation_id().as_str(), key.as_str(), value],
            )
            .map_err(|error| map_sqlite_error("insert state entry", error))?;
    }
    Ok(())
}

fn replace_state(transaction: &Transaction<'_>, state: &BackendState) -> Result<(), StateError> {
    let record = state.record();
    transaction
        .execute(
            "UPDATE installations
             SET plugin_id = ?1, state_schema_version = ?2, status = ?3,
                 revision = ?4, runtime_generation = ?5
             WHERE installation_id = ?6",
            params![
                record.plugin_id().as_str(),
                integer(record.state_schema_version().value(), "schema version")?,
                record.status().to_string(),
                integer(record.state_revision().value(), "record revision")?,
                integer(record.runtime_generation(), "runtime generation")?,
                record.installation_id().as_str(),
            ],
        )
        .map_err(|error| map_sqlite_error("update committed state record", error))?;
    transaction
        .execute(
            "DELETE FROM state_entries WHERE installation_id = ?1",
            params![record.installation_id().as_str()],
        )
        .map_err(|error| map_sqlite_error("replace state entries", error))?;
    for (key, value) in state.values() {
        transaction
            .execute(
                "INSERT INTO state_entries(installation_id, state_key, state_value)
                 VALUES (?1, ?2, ?3)",
                params![record.installation_id().as_str(), key.as_str(), value],
            )
            .map_err(|error| map_sqlite_error("insert committed state entry", error))?;
    }
    Ok(())
}

fn current_revision(
    transaction: &Transaction<'_>,
    installation: &InstallationId,
) -> Result<StateRevision, StateError> {
    let revision: Option<i64> = transaction
        .query_row(
            "SELECT revision FROM installations WHERE installation_id = ?1",
            params![installation.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| map_sqlite_error("read current revision", error))?;
    let revision = revision.ok_or_else(|| StateError::UnknownInstallation {
        installation: installation.clone(),
    })?;
    revision_from_integer(revision, "stored revision")
}

fn record_columns(row: &Row<'_>) -> rusqlite::Result<RecordColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn record_from_columns(columns: RecordColumns) -> Result<InstallationRecord, StateError> {
    let (installation, plugin, schema, status, revision, runtime_generation) = columns;
    let status = match status.as_str() {
        "Installed" => InstallationStatus::Installed,
        "PreparingState" => InstallationStatus::PreparingState,
        "Migrating" => InstallationStatus::Migrating,
        "Ready" => InstallationStatus::Ready,
        "MigrationFailed" => InstallationStatus::MigrationFailed,
        "Uninstalling" => InstallationStatus::Uninstalling,
        "RecoveryFailed" => InstallationStatus::RecoveryFailed,
        other => {
            return Err(StateError::Persistence(PersistenceError::StorageCorrupt {
                message: format!("unknown installation status '{other}'"),
            }));
        }
    };
    Ok(InstallationRecord::from_parts(
        InstallationId::new(installation),
        PluginId::new(plugin),
        StateSchemaVersion::new(unsigned_integer(schema, "schema version")?),
        status,
        StateRevision::new(unsigned_integer(revision, "revision")?),
        unsigned_integer(runtime_generation, "runtime generation")?,
    ))
}

fn integer(value: u64, name: &str) -> Result<i64, StateError> {
    i64::try_from(value).map_err(|_| {
        StateError::Persistence(PersistenceError::StorageIoFailure {
            operation: "encode".to_owned(),
            message: format!("{name} exceeds SQLite integer range"),
        })
    })
}

fn unsigned_integer(value: i64, name: &str) -> Result<u64, StateError> {
    u64::try_from(value).map_err(|_| {
        StateError::Persistence(PersistenceError::StorageCorrupt {
            message: format!("{name} contains negative value {value}"),
        })
    })
}

fn revision_from_integer(value: i64, name: &str) -> Result<StateRevision, StateError> {
    Ok(StateRevision::new(unsigned_integer(value, name)?))
}

fn open_error(operation: &str, error: rusqlite::Error) -> PersistenceError {
    PersistenceError::StorageOpenFailed {
        message: format!("{operation}: {error}"),
    }
}

fn map_sqlite_error(operation: &'static str, error: rusqlite::Error) -> StateError {
    let message = error.to_string();
    let persistence = match error {
        rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                PersistenceError::StorageBusy { message }
            }
            _ => PersistenceError::StorageIoFailure {
                operation: operation.to_owned(),
                message,
            },
        },
        rusqlite::Error::FromSqlConversionFailure(_, Type::Blob, source) => {
            PersistenceError::StorageCorrupt {
                message: format!("{operation}: {source}"),
            }
        }
        _ => PersistenceError::StorageIoFailure {
            operation: operation.to_owned(),
            message,
        },
    };
    StateError::Persistence(persistence)
}
