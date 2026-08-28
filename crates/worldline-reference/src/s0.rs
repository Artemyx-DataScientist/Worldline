use std::sync::Arc;

use worldline_kernel::{GrantLifetime, InMemoryStateBackend, Kernel, ResourceScope};

use crate::{
    ObservationBus,
    browser_like::{
        BrowserLikeConsumer, BrowserLikeProvider, capability_from_slot, navigate_capability,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S0Report {
    pub installation_id: String,
    pub state_before_restart: String,
    pub state_after_restart: String,
    pub old_runtime_id: String,
    pub new_runtime_id: String,
    pub old_runtime: String,
    pub new_runtime: String,
    pub first_result: String,
    pub restarted_result: String,
    pub observations: usize,
    pub old_runtime_grant_revoked: bool,
    pub new_runtime_did_not_inherit_authority: bool,
}

/// Runs the durable host-level proving slice over only public kernel APIs.
pub fn run() -> Result<S0Report, String> {
    let backend: Arc<dyn worldline_kernel::StateBackend> = Arc::new(InMemoryStateBackend::new());
    let bus = ObservationBus::new();
    let observed = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let observed_clone = Arc::clone(&observed);
    bus.subscribe(move |event| {
        observed_clone
            .lock()
            .map_err(|_| "observation lock is poisoned".to_owned())?
            .push(format!(
                "{}:{}:{}",
                event.id(),
                event.producer(),
                String::from_utf8_lossy(event.payload())
            ));
        Ok(())
    });

    let mut first_host =
        Kernel::with_state_backend(Arc::clone(&backend)).map_err(|error| error.to_string())?;
    let provider = first_host
        .register(BrowserLikeProvider::new(
            "reference.browser.provider",
            bus.clone(),
            "provider-a",
        ))
        .map_err(|error| error.to_string())?;
    let installation = first_host
        .installation_id_for_plugin(&provider)
        .ok_or_else(|| "provider installation was not created".to_owned())?;
    let old_runtime_id = first_host
        .runtime_id_for_plugin(&provider)
        .ok_or_else(|| "provider runtime id was not allocated".to_owned())?;
    let old_runtime = first_host
        .principal_for_plugin(&provider)
        .ok_or_else(|| "provider runtime was not activated".to_owned())?;

    let (consumer, consumer_slot) = BrowserLikeConsumer::new("reference.browser.consumer");
    let consumer_id = first_host
        .register(consumer)
        .map_err(|error| error.to_string())?;
    let consumer_installation = first_host
        .installation_id_for_plugin(&consumer_id)
        .ok_or_else(|| "consumer installation was not created".to_owned())?;
    let consumer_handle = capability_from_slot(&consumer_slot)
        .ok_or_else(|| "browser consumer did not receive its capability handle".to_owned())?;
    let consumer_principal = first_host
        .principal_for_plugin(&consumer_id)
        .ok_or_else(|| "consumer runtime was not activated".to_owned())?;
    first_host
        .create_root_grant(
            consumer_principal,
            navigate_capability().contract(),
            ["navigate"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .map_err(|error| error.to_string())?;

    let first_result = String::from_utf8(
        consumer_handle
            .invoke("navigate", b"https://worldline.example")
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let state_before_restart = read_activation_count(&first_host, &installation)?;

    let old_runtime_grant = first_host
        .create_root_grant(
            old_runtime.clone(),
            navigate_capability().contract(),
            ["navigate"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .map_err(|error| error.to_string())?;
    first_host
        .unregister(&provider)
        .map_err(|error| error.to_string())?;
    let old_runtime_grant_revoked = !first_host.is_grant_active(&old_runtime_grant);
    if !old_runtime_grant_revoked {
        return Err("unregister left old runtime authority active".to_owned());
    }
    drop(first_host);

    let mut restarted_host =
        Kernel::with_state_backend(Arc::clone(&backend)).map_err(|error| error.to_string())?;
    let restarted_provider = restarted_host
        .register_for_installation(
            BrowserLikeProvider::new("reference.browser.provider", bus.clone(), "provider-b"),
            &installation,
        )
        .map_err(|error| error.to_string())?;
    let new_runtime = restarted_host
        .principal_for_plugin(&restarted_provider)
        .ok_or_else(|| "restarted provider runtime was not activated".to_owned())?;
    let new_runtime_id = restarted_host
        .runtime_id_for_plugin(&restarted_provider)
        .ok_or_else(|| "restarted provider runtime id was not allocated".to_owned())?;
    if old_runtime == new_runtime {
        return Err("host restart reused the previous runtime principal".to_owned());
    }
    let state_after_restart = read_activation_count(&restarted_host, &installation)?;
    if state_after_restart == state_before_restart {
        return Err("restarted provider did not update installation state".to_owned());
    }

    let new_runtime_handle = restarted_host
        .capability_for(new_runtime.clone(), navigate_capability())
        .map_err(|error| error.to_string())?;
    let new_runtime_did_not_inherit_authority = matches!(
        new_runtime_handle.invoke("navigate", b"authority-check"),
        Err(worldline_kernel::CapabilityError::Denied {
            reason: worldline_kernel::DenialReason::NoGrant,
            ..
        })
    );
    if !new_runtime_did_not_inherit_authority {
        return Err("new runtime unexpectedly inherited old authority".to_owned());
    }

    let (restarted_consumer, restarted_consumer_slot) =
        BrowserLikeConsumer::new("reference.browser.consumer");
    let restarted_consumer_id = restarted_host
        .register_for_installation(restarted_consumer, &consumer_installation)
        .map_err(|error| error.to_string())?;
    let restarted_consumer_handle = capability_from_slot(&restarted_consumer_slot)
        .ok_or_else(|| "restarted consumer did not receive its capability handle".to_owned())?;
    let restarted_consumer_principal = restarted_host
        .principal_for_plugin(&restarted_consumer_id)
        .ok_or_else(|| "restarted consumer runtime was not activated".to_owned())?;
    restarted_host
        .create_root_grant(
            restarted_consumer_principal,
            navigate_capability().contract(),
            ["navigate"],
            ResourceScope::Any,
            false,
            GrantLifetime::Persistent,
        )
        .map_err(|error| error.to_string())?;
    let restarted_result = String::from_utf8(
        restarted_consumer_handle
            .invoke("navigate", b"https://worldline.example/restarted")
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let observations = observed
        .lock()
        .map_err(|_| "observation lock is poisoned".to_owned())?
        .len();
    Ok(S0Report {
        installation_id: installation.to_string(),
        state_before_restart,
        state_after_restart,
        old_runtime_id: old_runtime_id.to_string(),
        new_runtime_id: new_runtime_id.to_string(),
        old_runtime: old_runtime.to_string(),
        new_runtime: new_runtime.to_string(),
        first_result,
        restarted_result,
        observations,
        old_runtime_grant_revoked,
        new_runtime_did_not_inherit_authority,
    })
}

fn read_activation_count(
    kernel: &Kernel,
    installation: &worldline_kernel::InstallationId,
) -> Result<String, String> {
    let value = kernel
        .state_handle(installation)
        .map_err(|error| error.to_string())?
        .get("activation-count")
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "activation count is missing".to_owned())?;
    String::from_utf8(value).map_err(|error| error.to_string())
}
