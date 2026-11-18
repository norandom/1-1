//! An operator can check one saved host. All work shares its first deadline.
use crate::rdp::credential_probe::{CredentialOutcome, ProbeReason, probe_saved_host};
use crate::rdp::peer_identity::{ConfirmedPin, ConnectionTarget, pins::PinStore};
use crate::store::{
    connections::ConnectionConfig, credentials::CredentialStore, library::LibraryStore,
};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager, State};
use tokio::sync::{Mutex as AsyncMutex, RwLock, oneshot, watch};
use tokio::time::{Instant, sleep_until, timeout_at};
use uuid::Uuid;

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckedCredential {
    connection_id: String,
    revision: Uuid,
    #[serde(flatten)]
    result: CredentialOutcome,
}

struct Flight {
    result: watch::Sender<Option<CredentialOutcome>>,
    stopped: oneshot::Receiver<()>,
}

struct Revision {
    id: Uuid,
    flight: Option<Flight>,
}

struct HostChecks {
    edit: AsyncMutex<()>,
    revision: Mutex<Revision>,
}

impl HostChecks {
    fn new() -> Self {
        Self {
            edit: AsyncMutex::new(()),
            revision: Mutex::new(Revision {
                id: Uuid::new_v4(),
                flight: None,
            }),
        }
    }

    fn invalidate(&self) -> Option<oneshot::Receiver<()>> {
        let flight = {
            let mut revision = self.revision.lock().unwrap();
            revision.id = Uuid::new_v4();
            revision.flight.take()
        };
        flight.map(|Flight { result, stopped }| {
            drop(result);
            stopped
        })
    }
}

#[derive(Default)]
pub(crate) struct CredentialChecks {
    // Admission and tree-wide mutations use this gate. Active network and
    // blocking lookup work run outside it. Ordinary edits lock only one host.
    pub(super) library_gate: RwLock<()>,
    hosts: Mutex<HashMap<String, Arc<HostChecks>>>,
    publishing: Mutex<Publishing>,
}

#[derive(Default)]
struct Publishing {
    library: bool,
    hosts: HashSet<String>,
}

struct Publication<'a> {
    checks: &'a CredentialChecks,
    ids: Option<&'a [String]>,
}

impl Drop for Publication<'_> {
    fn drop(&mut self) {
        let mut publishing = self.checks.publishing.lock().unwrap();
        match self.ids {
            Some(ids) => {
                for id in ids {
                    publishing.hosts.remove(id);
                }
            }
            None => publishing.library = false,
        }
    }
}

impl CredentialChecks {
    // Writers already hold either the per-host edit lock or the exclusive
    // library gate. Mark the ambiguous interval before invalidation begins.
    fn publication<'a>(&'a self, ids: Option<&'a [String]>) -> Publication<'a> {
        let mut publishing = self.publishing.lock().unwrap();
        match ids {
            Some(ids) => publishing.hosts.extend(ids.iter().cloned()),
            None => publishing.library = true,
        }
        Publication { checks: self, ids }
    }

    fn host(&self, id: &str) -> Arc<HostChecks> {
        self.hosts
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(HostChecks::new()))
            .clone()
    }

    pub(super) async fn edit<T>(&self, id: &str, change: impl Future<Output = T>) -> T {
        let _library = self.library_gate.read().await;
        let host = self.host(id);
        let _writer = host.edit.lock().await;
        let affected = [id.to_owned()];
        let _publishing = self.publication(Some(&affected));
        if let Some(stopped) = host.invalidate() {
            // Wait for the old network future to drop before changing the identity.
            let _ = stopped.await;
        }
        change.await
    }

    /// Call under the exclusive library gate before a reload or cascade.
    pub(super) async fn publish_library<T>(
        &self,
        ids: Option<&[String]>,
        change: impl Future<Output = T>,
    ) -> T {
        let _publishing = self.publication(ids);
        let retired: Vec<_> = {
            let mut hosts = self.hosts.lock().unwrap();
            match ids {
                Some(ids) => ids.iter().filter_map(|id| hosts.remove(id)).collect(),
                None => hosts.drain().map(|(_, host)| host).collect(),
            }
        };
        let pending: Vec<_> = retired
            .iter()
            .filter_map(|host| host.invalidate())
            .collect();
        for closed in pending {
            let _ = closed.await;
        }
        change.await
    }

    async fn check<F>(&self, id: &str, work: F) -> Option<CheckedCredential>
    where
        F: Future<Output = CredentialOutcome> + Send + 'static,
    {
        let deadline = Instant::now() + CHECK_TIMEOUT;
        let timed_out = CredentialOutcome::UnableToVerify(ProbeReason::Timeout);
        let response = |revision, result| CheckedCredential {
            connection_id: id.to_owned(),
            revision,
            result,
        };
        // Capture the revision when the operator clicks, including while a
        // reload or host edit is still holding admission.
        let (host, selected) = {
            let publishing = self.publishing.lock().unwrap();
            if publishing.library || publishing.hosts.contains(id) {
                return None;
            }
            let host = self.host(id);
            let selected = host.revision.lock().unwrap().id;
            (host, selected)
        };
        let timeout_result = || {
            (host.revision.lock().unwrap().id == selected).then(|| response(selected, timed_out))
        };
        let Ok(library) = timeout_at(deadline, self.library_gate.read()).await else {
            return timeout_result();
        };
        let Ok(edit) = timeout_at(deadline, host.edit.lock()).await else {
            return timeout_result();
        };
        let (revision_id, mut result) = {
            let mut revision = host.revision.lock().unwrap();
            if revision.id != selected {
                return None;
            }
            let revision_id = revision.id;
            if let Some(pending) = &revision.flight {
                (revision_id, pending.result.subscribe())
            } else {
                let (sender, receiver) = watch::channel(None);
                let (stopped, closed) = oneshot::channel();
                revision.flight = Some(Flight {
                    result: sender,
                    stopped: closed,
                });
                let slot = host.clone();
                let mut cancelled = receiver.clone();
                tokio::spawn(async move {
                    let value = tokio::select! {
                        biased;
                        _ = cancelled.changed() => return,
                        _ = sleep_until(deadline) => timed_out,
                        outcome = work => outcome,
                    };
                    let mut state = slot.revision.lock().unwrap();
                    if state.id == revision_id
                        && let Some(flight) = state.flight.take()
                    {
                        flight.result.send_replace(Some(value));
                    }
                    let _ = stopped.send(());
                });
                (revision_id, receiver)
            }
        };
        drop(edit);
        drop(library);
        result.wait_for(Option::is_some).await.ok()?;
        let outcome = (*result.borrow())?;
        if host.revision.lock().unwrap().id != revision_id {
            return None;
        }
        if matches!(
            outcome,
            CredentialOutcome::UnableToVerify(ProbeReason::UnknownHost)
        ) {
            // Only retain revisions for real saved hosts. Never retire a slot
            // another caller is already editing or using.
            let mut hosts = self.hosts.lock().unwrap();
            if Arc::strong_count(&host) == 2 && hosts.get(id).is_some_and(|s| Arc::ptr_eq(s, &host))
            {
                hosts.remove(id);
            }
        }
        Some(response(revision_id, outcome))
    }
}

async fn check_saved(
    checks: &CredentialChecks,
    id: &str,
    load: impl FnOnce() -> Result<SavedInputs, ProbeReason> + Send + 'static,
    current: impl FnOnce(&Arc<ConnectionConfig>) -> bool + Send + 'static,
) -> Option<CheckedCredential> {
    checks
        .check(id, async move {
            match tokio::task::spawn_blocking(load).await {
                Ok(Ok(saved)) => {
                    let outcome =
                        probe_saved_host(&saved.host, &saved.password, saved.pin.as_ref()).await;
                    let host = saved.host;
                    // Another app instance may have replaced the persisted host
                    // while the verified exchange was in progress.
                    match tokio::task::spawn_blocking(move || current(&host)).await {
                        Ok(true) => outcome,
                        Ok(false) => CredentialOutcome::UnableToVerify(ProbeReason::HostChanged),
                        Err(_) => CredentialOutcome::UnableToVerify(ProbeReason::StoreUnavailable),
                    }
                }
                Ok(Err(reason)) => CredentialOutcome::UnableToVerify(reason),
                Err(_) => CredentialOutcome::UnableToVerify(ProbeReason::StoreUnavailable),
            }
        })
        .await
}

struct SavedInputs {
    host: Arc<ConnectionConfig>,
    password: String,
    pin: Option<ConfirmedPin>,
}

fn read_saved_inputs(
    id: &str,
    library: &LibraryStore,
    credentials: &CredentialStore,
    pins: &PinStore,
) -> Result<SavedInputs, ProbeReason> {
    let host =
        library
            .select_host_for_review(id)
            .map_err(|_| match library.get_connection(id) {
                Ok(None) => ProbeReason::UnknownHost,
                _ => ProbeReason::StoreUnavailable,
            })?;
    if host.mock {
        return Err(ProbeReason::UnsupportedAuthentication);
    }
    if host.username.is_empty() {
        return Err(ProbeReason::MissingIdentity);
    }
    let target =
        ConnectionTarget::new(&host.host, host.port).map_err(|_| ProbeReason::Unreachable)?;
    let password = credentials
        .get_password(id)
        .map_err(|_| ProbeReason::KeyringUnavailable)?
        .filter(|value| !value.is_empty())
        .ok_or(ProbeReason::MissingCredential)?;
    let pin = pins
        .lookup(&target.endpoint)
        .map_err(|_| ProbeReason::StoreUnavailable)?;
    library
        .with_reviewed_host(&host, || Ok(()))
        .map_err(|_| ProbeReason::HostChanged)?;
    Ok(SavedInputs {
        host,
        password,
        pin,
    })
}

#[tauri::command]
pub(crate) async fn test_saved_credentials(
    connection_id: String,
    app: AppHandle,
    checks: State<'_, CredentialChecks>,
) -> Result<Option<CheckedCredential>, ()> {
    let id = connection_id.clone();
    let verification = app.clone();
    Ok(check_saved(
        &checks,
        &connection_id,
        move || {
            read_saved_inputs(
                &id,
                &app.state::<LibraryStore>(),
                &app.state::<CredentialStore>(),
                &app.state::<PinStore>(),
            )
        },
        move |host| {
            verification
                .state::<LibraryStore>()
                .with_reviewed_host(host, || Ok(()))
                .is_ok()
        },
    )
    .await)
}

#[cfg(test)]
mod tests;
