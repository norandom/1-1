use super::{ConfirmedPin, ConnectionTarget, Endpoint, ObservedPeer, discover, endpoint_key};
use crate::store::{
    atomic_write::write_atomic, connections::ConnectionConfig, library::LibraryStore,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

const REVIEW_LIFETIME: Duration = Duration::from_secs(120);
const STALE: &str = "This review has expired or closed. Review the server identity again.";

#[cfg(test)]
mod tests;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IdentityReview {
    pub(crate) selection: Uuid,
    pub(crate) endpoint: String,
    pub(crate) previous_fingerprint: Option<String>,
    pub(crate) observed_fingerprint: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinRecord {
    revision: Uuid,
    spki_sha256: [u8; 32],
}

struct Draft {
    host: Arc<ConnectionConfig>,
    peer: ObservedPeer,
    previous: Option<PinRecord>,
    expires: Instant,
}

pub(crate) struct PinStore {
    path: PathBuf,
    drafts: Mutex<HashMap<Uuid, Draft>>,
}

fn fingerprint(hash: &[u8; 32]) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
    format!("SHA256:{}", STANDARD_NO_PAD.encode(hash))
}

impl PinStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            drafts: Mutex::default(),
        }
    }

    fn read(&self) -> io::Result<BTreeMap<String, PinRecord>> {
        match fs::read(&self.path) {
            Ok(data) => serde_json::from_slice(&data)
                .map_err(|_| io::Error::other("The server identity file is invalid.")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(_) => Err(io::Error::other(
                "Could not read the saved server identities.",
            )),
        }
    }

    /// Used by credential and share checks before verifying their own TLS sockets.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "consumed by the credential probe and share attachment tasks"
        )
    )]
    pub(crate) fn lookup(&self, endpoint: &Endpoint) -> io::Result<Option<ConfirmedPin>> {
        Ok(self.read()?.get(&endpoint_key(endpoint)).map(|pin| {
            ConfirmedPin::from_operator_confirmation(ObservedPeer {
                endpoint: endpoint.clone(),
                spki_sha256: pin.spki_sha256,
            })
        }))
    }

    pub(crate) async fn review(
        &self,
        library: &LibraryStore,
        id: &str,
    ) -> io::Result<IdentityReview> {
        let host = library.select_host_for_review(id)?;
        let target = ConnectionTarget::new(&host.host, host.port)?;
        let peer = discover(&target).await?;
        self.offer(library, &host, peer)
    }

    fn offer(
        &self,
        library: &LibraryStore,
        host: &Arc<ConnectionConfig>,
        peer: ObservedPeer,
    ) -> io::Result<IdentityReview> {
        library.with_reviewed_host(host, || {
            let endpoint = endpoint_key(&peer.endpoint);
            let previous = self.read()?.remove(&endpoint);
            let selection = Uuid::new_v4();
            let review = IdentityReview {
                selection,
                endpoint,
                previous_fingerprint: previous.as_ref().map(|p| fingerprint(&p.spki_sha256)),
                observed_fingerprint: fingerprint(&peer.spki_sha256),
            };
            let now = Instant::now();
            let mut drafts = self.drafts.lock().expect("identity review lock poisoned");
            drafts.retain(|_, draft| draft.expires > now);
            drafts.insert(
                selection,
                Draft {
                    host: host.clone(),
                    peer,
                    previous,
                    expires: now + REVIEW_LIFETIME,
                },
            );
            Ok(review)
        })
    }

    pub(crate) fn cancel(&self, selection: Uuid) {
        self.drafts
            .lock()
            .expect("identity review lock poisoned")
            .remove(&selection);
    }

    pub(crate) fn confirm(&self, library: &LibraryStore, selection: Uuid) -> io::Result<()> {
        let draft = self
            .drafts
            .lock()
            .expect("identity review lock poisoned")
            .remove(&selection)
            .ok_or_else(|| io::Error::other(STALE))?;
        library.with_reviewed_host(&draft.host, || {
            // Use a separate, stable lock file: the atomic rename replaces the pin file itself.
            let lock = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(self.path.with_extension("lock"))
                .map_err(|_| io::Error::other("Could not save the server identity."))?;
            lock.try_lock().map_err(|_| {
                io::Error::other("The server identity store is busy. Review again.")
            })?;
            if Instant::now() >= draft.expires {
                return Err(io::Error::other(STALE));
            }
            let mut pins = self.read()?;
            let key = endpoint_key(&draft.peer.endpoint);
            if pins.get(&key) != draft.previous.as_ref() {
                return Err(io::Error::other(
                    "The saved fingerprint changed. Review the server identity again.",
                ));
            }
            pins.insert(
                key,
                PinRecord {
                    revision: Uuid::new_v4(),
                    spki_sha256: draft.peer.spki_sha256,
                },
            );
            let data = serde_json::to_vec_pretty(&pins)?;
            write_atomic(&self.path, &data)
                .map_err(|_| io::Error::other("Could not save the server identity."))
        })
    }
}
