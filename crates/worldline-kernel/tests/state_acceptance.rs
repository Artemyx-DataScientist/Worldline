use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use worldline_kernel::{
    ActivationContext, CapabilityId, GrantLifetime, InstallationStatus, InterfaceVersion, Kernel,
    MigrationError, MigrationId, NoopRuntime, Plugin, PluginDefinition, PluginError, PluginRuntime,
    PrincipalKind, ResourceScope, StateError, StateKey, StateMigration, StateSchemaVersion,
    TrajectoryEventKind,
};

fn schema(value: u64) -> StateSchemaVersion {
    StateSchemaVersion::new(value)
}

fn capability() -> CapabilityId {
    CapabilityId::new(
        "worldline.state-test",
        "service",
        InterfaceVersion::new(1, 0),
    )
}

struct StatePlugin {
    definition: PluginDefinition,
    activation_count: Arc<AtomicUsize>,
    write_key: Option<StateKey>,
    write_value: Vec<u8>,
}

impl StatePlugin {
    fn new(definition: PluginDefinition) -> Self {
        Self {
            definition,
            activation_count: Arc::new(AtomicUsize::new(0)),
            write_key: None,
            write_value: Vec::new(),
        }
    }

    fn writes(mut self, key: impl Into<StateKey>, value: impl AsRef<[u8]>) -> Self {
        self.write_key = Some(key.into());
        self.write_value = value.as_ref().to_vec();
        self
    }
}

impl Plugin for StatePlugin {
    fn definition(&self) -> &PluginDefinition {
        &self.definition
    }

    fn activate(
        &self,
        context: &mut ActivationContext,
    ) -> Result<Box<dyn PluginRuntime>, PluginError> {
        self.activation_count.fetch_add(1, Ordering::SeqCst);
        if let Some(key) = &self.write_key {
            let mut transaction = context
                .state()
                .transaction()
                .map_err(|error| PluginError::new(error.to_string()))?;
            transaction
                .put(key.clone(), &self.write_value)
                .map_err(|error| PluginError::new(error.to_string()))?;
            transaction
                .commit()
                .map_err(|error| PluginError::new(error.to_string()))?;
        }
        Ok(Box::new(NoopRuntime))
    }
}

fn install(kernel: &mut Kernel, plugin: &str, version: u64) -> worldline_kernel::InstallationId {
    kernel
        .create_installation(plugin, schema(version))
        .expect("installation must be created")
}

fn put(kernel: &Kernel, installation: &worldline_kernel::InstallationId, key: &str, value: &[u8]) {
    let handle = kernel
        .state_handle(installation)
        .expect("installation must be ready");
    let mut transaction = handle.transaction().expect("transaction must start");
    transaction.put(key, value).expect("state write must work");
    transaction.commit().expect("state commit must work");
}

#[test]
fn installation_identity_is_distinct_from_runtime() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "identity-plugin", 0);
    let plugin = kernel
        .register_for_installation(
            StatePlugin::new(PluginDefinition::new("identity-plugin")),
            &installation,
        )
        .expect("plugin registration must succeed");
    let runtime = kernel
        .principal_for_plugin(&plugin)
        .expect("active plugin must have a runtime principal");
    let installation_principal = kernel
        .principal_for_installation(&installation)
        .expect("installation must have a principal");

    assert_ne!(installation.as_str(), runtime.as_str());
    assert_ne!(runtime, installation_principal);
    assert_eq!(
        kernel
            .principal(&installation_principal)
            .expect("installation principal must be registered")
            .kind(),
        PrincipalKind::PluginInstallation
    );
}

#[test]
fn two_installations_of_one_plugin_are_isolated() {
    let mut kernel = Kernel::new();
    let first = install(&mut kernel, "same-plugin", 0);
    let second = install(&mut kernel, "same-plugin", 0);
    put(&kernel, &first, "value", b"first");
    put(&kernel, &second, "value", b"second");

    assert_eq!(
        kernel.state_handle(&first).unwrap().get("value").unwrap(),
        Some(b"first".to_vec())
    );
    assert_eq!(
        kernel.state_handle(&second).unwrap().get("value").unwrap(),
        Some(b"second".to_vec())
    );
}

#[test]
fn runtime_binding_rejects_another_installation() {
    let mut kernel = Kernel::new();
    let first = install(&mut kernel, "bound-plugin", 0);
    let second = install(&mut kernel, "bound-plugin", 0);
    let plugin = kernel
        .register_for_installation(
            StatePlugin::new(PluginDefinition::new("bound-plugin")),
            &first,
        )
        .unwrap();

    assert!(matches!(
        kernel.state_handle_for_plugin(&plugin, &second),
        Err(StateError::RuntimeInstallationMismatch { expected, actual })
            if expected == first && actual == second
    ));
}

#[test]
fn transactions_are_atomic_and_rollback_is_invisible() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "transactions", 0);
    put(&kernel, &installation, "old", b"before");
    let handle = kernel.state_handle(&installation).unwrap();
    let mut transaction = handle.transaction().unwrap();
    transaction.put("a", b"one").unwrap();
    transaction.put("b", b"two").unwrap();
    transaction.delete("old").unwrap();
    assert_eq!(handle.get("old").unwrap(), Some(b"before".to_vec()));
    assert_eq!(handle.get("a").unwrap(), None);
    transaction.commit().unwrap();
    assert_eq!(handle.get("old").unwrap(), None);
    assert_eq!(handle.get("a").unwrap(), Some(b"one".to_vec()));
    assert_eq!(handle.get("b").unwrap(), Some(b"two".to_vec()));

    let mut rollback = handle.transaction().unwrap();
    rollback.put("a", b"not-committed").unwrap();
    rollback.rollback().unwrap();
    assert_eq!(handle.get("a").unwrap(), Some(b"one".to_vec()));
}

#[test]
fn injected_commit_failure_preserves_previous_state() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "commit-failure", 4);
    put(&kernel, &installation, "stable", b"old");
    let handle = kernel.state_handle(&installation).unwrap();
    let mut transaction = handle.transaction().unwrap();
    transaction.put("stable", b"new").unwrap();
    transaction.put("extra", b"must-not-appear").unwrap();
    kernel.fail_next_state_commit();
    assert!(matches!(
        transaction.commit(),
        Err(StateError::TransactionCommitFailed { .. })
    ));
    assert_eq!(handle.get("stable").unwrap(), Some(b"old".to_vec()));
    assert_eq!(handle.get("extra").unwrap(), None);
    assert_eq!(
        kernel
            .installation(&installation)
            .unwrap()
            .state_schema_version(),
        schema(4)
    );
}

#[test]
fn unregister_preserves_state_and_new_runtime_gets_new_authority() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "restart-plugin", 0);
    let definition = PluginDefinition::new("restart-plugin");
    let first = kernel
        .register_for_installation(
            StatePlugin::new(definition.clone()).writes("counter", b"one"),
            &installation,
        )
        .unwrap();
    let old_runtime = kernel.principal_for_plugin(&first).unwrap();
    let grant = kernel
        .create_root_grant(
            old_runtime.clone(),
            capability().contract(),
            ["read"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .unwrap();
    kernel.unregister(&first).unwrap();
    assert_eq!(
        kernel
            .state_handle(&installation)
            .unwrap()
            .get("counter")
            .unwrap(),
        Some(b"one".to_vec())
    );

    let second = kernel
        .register_for_installation(StatePlugin::new(definition), &installation)
        .unwrap();
    let new_runtime = kernel.principal_for_plugin(&second).unwrap();
    assert_ne!(old_runtime, new_runtime);
    assert!(!kernel.is_grant_active(&grant));
    assert!(
        kernel
            .grant(&grant)
            .unwrap()
            .subject()
            .as_str()
            .contains("plugin-runtime:restart-plugin:")
    );
}

#[test]
fn shared_backend_restart_recovers_state_without_runtime_authority() {
    let backend = Arc::new(worldline_kernel::InMemoryStateBackend::new());
    let installation;
    {
        let mut first_kernel = Kernel::with_state_backend(backend.clone());
        installation = install(&mut first_kernel, "backend-restart", 0);
        put(&first_kernel, &installation, "persistent", b"survives");
    }
    let second_kernel = Kernel::with_state_backend(backend);
    assert_eq!(
        second_kernel
            .state_handle(&installation)
            .unwrap()
            .get("persistent")
            .unwrap(),
        Some(b"survives".to_vec())
    );
    assert!(
        second_kernel
            .principal_for_plugin(&"backend-restart".into())
            .is_none()
    );
}

#[test]
fn same_schema_skips_migration() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "same-schema", 2);
    let called = Arc::new(AtomicUsize::new(0));
    let migration_called = Arc::clone(&called);
    let definition = PluginDefinition::new("same-schema")
        .with_state_schema_version(schema(2))
        .with_state_migration(StateMigration::new(
            "never",
            schema(1),
            schema(2),
            move |_| {
                migration_called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        ));
    kernel
        .register_for_installation(StatePlugin::new(definition), &installation)
        .unwrap();
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert!(!kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::MigrationStarted { installation: id, .. } if id == &installation
        )
    }));
}

#[test]
fn multistep_migration_is_deterministic_and_precedes_activation() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "multi-step", 1);
    put(&kernel, &installation, "value", b"v1");
    let steps = Arc::new(Mutex::new(Vec::new()));
    let first_steps = Arc::clone(&steps);
    let second_steps = Arc::clone(&steps);
    let definition = PluginDefinition::new("multi-step")
        .with_state_schema_version(schema(3))
        .with_state_migration(StateMigration::new(
            MigrationId::new("one-to-two"),
            schema(1),
            schema(2),
            move |context| {
                first_steps.lock().unwrap().push("one-to-two");
                assert_eq!(context.from_schema(), schema(1));
                context
                    .put("value", b"v2")
                    .map_err(|error| MigrationError::new(error.to_string()))
            },
        ))
        .with_state_migration(StateMigration::new(
            MigrationId::new("two-to-three"),
            schema(2),
            schema(3),
            move |context| {
                second_steps.lock().unwrap().push("two-to-three");
                assert_eq!(context.to_schema(), schema(3));
                context
                    .put("value", b"v3")
                    .map_err(|error| MigrationError::new(error.to_string()))
            },
        ));
    kernel
        .register_for_installation(StatePlugin::new(definition), &installation)
        .unwrap();
    assert_eq!(*steps.lock().unwrap(), vec!["one-to-two", "two-to-three"]);
    assert_eq!(
        kernel
            .installation(&installation)
            .unwrap()
            .state_schema_version(),
        schema(3)
    );
    assert_eq!(
        kernel
            .state_handle(&installation)
            .unwrap()
            .get("value")
            .unwrap(),
        Some(b"v3".to_vec())
    );
    let events = kernel.trajectory();
    let committed = events
        .iter()
        .position(|event| {
            matches!(
                event.kind(),
                TrajectoryEventKind::MigrationCommitted { installation: id, .. } if id == &installation
            )
        })
        .unwrap();
    let ready = events
        .iter()
        .enumerate()
        .find(|(index, event)| {
            *index > committed
                && matches!(
                    event.kind(),
                    TrajectoryEventKind::InstallationReady { installation: id, schema: ready_schema } if id == &installation && *ready_schema == schema(3)
                )
        })
        .map(|(index, _)| index)
        .unwrap();
    let activated = events
        .iter()
        .position(|event| {
            event.plugin().as_str() == "multi-step"
                && matches!(event.kind(), TrajectoryEventKind::Activated)
        })
        .unwrap();
    assert!(committed < ready && ready < activated);
}

#[test]
fn failed_migration_rolls_back_state_and_blocks_activation() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "failed-migration", 1);
    put(&kernel, &installation, "value", b"before");
    let definition = PluginDefinition::new("failed-migration")
        .with_state_schema_version(schema(3))
        .with_state_migration(StateMigration::new(
            "one-to-two",
            schema(1),
            schema(2),
            |context| {
                context
                    .put("value", b"partial")
                    .map_err(|error| MigrationError::new(error.to_string()))
            },
        ))
        .with_state_migration(StateMigration::new(
            "two-to-three",
            schema(2),
            schema(3),
            |_| Err(MigrationError::new("intentional failure")),
        ));
    let plugin = StatePlugin::new(definition);
    let activated_counter = Arc::clone(&plugin.activation_count);
    kernel
        .register_for_installation(plugin, &installation)
        .unwrap();
    assert_eq!(activated_counter.load(Ordering::SeqCst), 0);
    assert_eq!(
        kernel.installation(&installation).unwrap().status(),
        InstallationStatus::MigrationFailed
    );
    assert_eq!(
        kernel
            .installation(&installation)
            .unwrap()
            .state_schema_version(),
        schema(1)
    );
    assert_eq!(
        kernel
            .state_handle(&installation)
            .unwrap()
            .get("value")
            .unwrap(),
        Some(b"before".to_vec())
    );
}

#[test]
fn failed_migration_waits_for_explicit_registration_retry() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "retry-migration", 1);
    let attempts = Arc::new(AtomicUsize::new(0));
    let failing_attempts = Arc::clone(&attempts);
    let failing_definition = PluginDefinition::new("retry-migration")
        .with_state_schema_version(schema(2))
        .with_state_migration(StateMigration::new(
            "fail-once",
            schema(1),
            schema(2),
            move |_| {
                failing_attempts.fetch_add(1, Ordering::SeqCst);
                Err(MigrationError::new("retry required"))
            },
        ));
    let plugin = kernel
        .register_for_installation(StatePlugin::new(failing_definition), &installation)
        .unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    kernel.reconcile();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    kernel.unregister(&plugin).unwrap();

    let successful_definition = PluginDefinition::new("retry-migration")
        .with_state_schema_version(schema(2))
        .with_state_migration(StateMigration::new(
            "retry-success",
            schema(1),
            schema(2),
            |_| Ok(()),
        ));
    kernel
        .register_for_installation(StatePlugin::new(successful_definition), &installation)
        .unwrap();
    assert_eq!(
        kernel.installation(&installation).unwrap().status(),
        InstallationStatus::Ready
    );
    assert_eq!(
        kernel
            .installation(&installation)
            .unwrap()
            .state_schema_version(),
        schema(2)
    );
}

#[test]
fn missing_and_ambiguous_paths_are_rejected() {
    let mut kernel = Kernel::new();
    let missing = install(&mut kernel, "missing-path", 1);
    let missing_definition = PluginDefinition::new("missing-path")
        .with_state_schema_version(schema(3))
        .with_state_migration(StateMigration::new(
            "one-to-two",
            schema(1),
            schema(2),
            |_| Ok(()),
        ));
    kernel
        .register_for_installation(StatePlugin::new(missing_definition), &missing)
        .unwrap();
    assert_eq!(
        kernel.installation(&missing).unwrap().status(),
        InstallationStatus::MigrationFailed
    );
    assert!(kernel.trajectory().iter().any(|event| {
        matches!(
            event.kind(),
            TrajectoryEventKind::MigrationFailed { installation, .. } if installation == &missing
        )
    }));

    let ambiguous = install(&mut kernel, "ambiguous-path", 1);
    let ambiguous_definition = PluginDefinition::new("ambiguous-path")
        .with_state_schema_version(schema(4))
        .with_state_migration(StateMigration::new(
            "one-to-two",
            schema(1),
            schema(2),
            |_| Ok(()),
        ))
        .with_state_migration(StateMigration::new(
            "two-to-four",
            schema(2),
            schema(4),
            |_| Ok(()),
        ))
        .with_state_migration(StateMigration::new(
            "one-to-three",
            schema(1),
            schema(3),
            |_| Ok(()),
        ))
        .with_state_migration(StateMigration::new(
            "three-to-four",
            schema(3),
            schema(4),
            |_| Ok(()),
        ));
    kernel
        .register_for_installation(StatePlugin::new(ambiguous_definition), &ambiguous)
        .unwrap();
    assert_eq!(
        kernel.installation(&ambiguous).unwrap().status(),
        InstallationStatus::MigrationFailed
    );
    assert_eq!(
        kernel
            .installation(&ambiguous)
            .unwrap()
            .state_schema_version(),
        schema(1)
    );
}

#[test]
fn upgrade_edges_do_not_imply_downgrade_but_explicit_downgrade_works() {
    let mut kernel = Kernel::new();
    let rejected = install(&mut kernel, "implicit-downgrade", 3);
    let rejected_definition = PluginDefinition::new("implicit-downgrade")
        .with_state_schema_version(schema(2))
        .with_state_migration(StateMigration::new(
            "two-to-three",
            schema(2),
            schema(3),
            |_| Ok(()),
        ));
    kernel
        .register_for_installation(StatePlugin::new(rejected_definition), &rejected)
        .unwrap();
    assert_eq!(
        kernel.installation(&rejected).unwrap().status(),
        InstallationStatus::MigrationFailed
    );

    let explicit = install(&mut kernel, "explicit-downgrade", 3);
    let explicit_definition = PluginDefinition::new("explicit-downgrade")
        .with_state_schema_version(schema(2))
        .with_state_migration(StateMigration::new(
            "three-to-two",
            schema(3),
            schema(2),
            |_| Ok(()),
        ));
    kernel
        .register_for_installation(StatePlugin::new(explicit_definition), &explicit)
        .unwrap();
    assert_eq!(
        kernel
            .installation(&explicit)
            .unwrap()
            .state_schema_version(),
        schema(2)
    );
    assert_eq!(
        kernel.installation(&explicit).unwrap().status(),
        InstallationStatus::Ready
    );
}

#[test]
fn unregister_is_not_uninstall_and_uninstall_deletes_state() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "uninstallable", 0);
    let plugin = kernel
        .register_for_installation(
            StatePlugin::new(PluginDefinition::new("uninstallable")).writes("key", b"value"),
            &installation,
        )
        .unwrap();
    kernel.unregister(&plugin).unwrap();
    assert!(kernel.installation(&installation).is_some());
    assert_eq!(
        kernel
            .state_handle(&installation)
            .unwrap()
            .get("key")
            .unwrap(),
        Some(b"value".to_vec())
    );
    kernel.uninstall(&installation).unwrap();
    assert!(kernel.installation(&installation).is_none());
    assert!(matches!(
        kernel.state_handle(&installation),
        Err(StateError::UnknownInstallation { .. })
    ));
}

#[test]
fn failed_uninstall_preserves_record_and_state() {
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "failed-uninstall", 0);
    put(&kernel, &installation, "key", b"preserve");
    kernel.fail_next_state_delete();
    assert!(matches!(
        kernel.uninstall(&installation),
        Err(worldline_kernel::KernelError::State(
            StateError::UninstallFailed { .. }
        ))
    ));
    assert_eq!(
        kernel
            .state_handle(&installation)
            .unwrap()
            .get("key")
            .unwrap(),
        Some(b"preserve".to_vec())
    );
    assert_eq!(
        kernel.installation(&installation).unwrap().status(),
        InstallationStatus::Ready
    );
    kernel.uninstall(&installation).unwrap();
}

#[test]
fn reinstall_after_uninstall_gets_a_new_identity() {
    let mut kernel = Kernel::new();
    let first = install(&mut kernel, "new-identity", 0);
    kernel.uninstall(&first).unwrap();
    let second = install(&mut kernel, "new-identity", 0);
    assert_ne!(first, second);
}

#[test]
fn state_trajectory_never_contains_raw_state_values() {
    let marker = "state-secret-marker-that-must-not-leak";
    let mut kernel = Kernel::new();
    let installation = install(&mut kernel, "trajectory-state", 0);
    put(&kernel, &installation, "secret", marker.as_bytes());
    let events = format!("{:?}", kernel.trajectory());
    assert!(!events.contains(marker));
}
