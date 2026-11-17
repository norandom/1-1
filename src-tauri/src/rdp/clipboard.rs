use std::{collections::VecDeque, env};

use crate::store::connections::ClipboardDirection;
use ironrdp_cliprdr::{
    backend::{ClipboardMessage, CliprdrBackend},
    pdu::{
        ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
        FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    },
};
use ironrdp_core::IntoOwned;
pub type ClipboardReply = tokio::sync::oneshot::Sender<Result<(), String>>;
pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 1024 * 1024;
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClipboardContext {
    pub epoch: String,
    pub direction: ClipboardDirection,
}

#[derive(Debug)]
pub struct SessionClipboardBackend {
    temporary_directory: String,
    pending_messages: VecDeque<ClipboardMessage>,
    local_text: Option<String>,
    remote_text_update: Option<String>,
    pending_remote_format: Option<ClipboardFormatId>,
    direction: ClipboardDirection,
    local_waiter: Option<ClipboardReply>,
    remote_waiter: Option<ClipboardReply>,
    remote_format: Option<ClipboardFormatId>,
    remote_revision: u64,
    pending_revision: u64,
    failure: Option<String>,
    blocked: bool,
    bootstrap_pending: bool,
}

impl SessionClipboardBackend {
    pub fn new() -> Self {
        Self::with_direction(ClipboardDirection::default())
    }

    pub fn with_direction(direction: ClipboardDirection) -> Self {
        Self {
            temporary_directory: env::temp_dir().to_string_lossy().into_owned(),
            pending_messages: VecDeque::new(),
            local_text: None,
            remote_text_update: None,
            pending_remote_format: None,
            direction,
            local_waiter: None,
            remote_waiter: None,
            remote_format: None,
            remote_revision: 0,
            pending_revision: 0,
            failure: None,
            blocked: false,
            bootstrap_pending: false,
        }
    }

    pub fn begin_local(&mut self, text: String, reply: ClipboardReply) {
        let error = if self.blocked {
            Some("Clipboard channel needs a reconnect.")
        } else if self.bootstrap_pending {
            Some("Clipboard initialization is awaiting acknowledgement. Retry shortly.")
        } else if !self.direction.sends() {
            Some(
                "Sending clipboard is disabled for this session. Reconnect after changing host settings.",
            )
        } else if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
            Some("Clipboard text exceeds the 1 MiB limit.")
        } else if self.local_waiter.is_some() {
            Some(
                "A clipboard announcement is still awaiting acknowledgement. Retry after it completes, or reconnect.",
            )
        } else {
            None
        };
        if let Some(error) = error {
            let _ = reply.send(Err(error.into()));
            return;
        }
        self.local_waiter = Some(reply);
        self.set_local_text(text);
    }

    pub fn begin_remote(&mut self, reply: ClipboardReply) {
        let error = if self.blocked {
            Some("Clipboard channel needs a reconnect.")
        } else if !self.direction.receives() {
            Some(
                "Receiving clipboard is disabled for this session. Reconnect after changing host settings.",
            )
        } else if self.pending_remote_format.is_some() {
            Some(
                "A remote clipboard response is still outstanding. Retry after it arrives, or reconnect.",
            )
        } else if self.remote_format.is_none() {
            Some("No remote text format is available. Copy text on the remote desktop and retry.")
        } else {
            None
        };
        if let Some(error) = error {
            let _ = reply.send(Err(error.into()));
            return;
        }
        self.remote_waiter = Some(reply);
        self.request_remote();
    }

    fn request_remote(&mut self) {
        if !self.blocked
            && self.direction.receives()
            && self.pending_remote_format.is_none()
            && let Some(format) = self.remote_format
        {
            self.pending_remote_format = Some(format);
            self.pending_revision = self.remote_revision;
            self.pending_messages
                .push_back(ClipboardMessage::SendInitiatePaste(format));
        }
    }

    pub fn fail_pending(&mut self, reason: &str) {
        self.blocked = true;
        self.remote_revision = self.remote_revision.wrapping_add(1);
        self.pending_messages.clear();
        for waiter in [self.local_waiter.take(), self.remote_waiter.take()]
            .into_iter()
            .flatten()
        {
            let _ = waiter.send(Err(reason.into()));
        }
        self.failure = Some(reason.into());
    }
    pub fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }

    pub fn set_local_text(&mut self, text: String) {
        if !self.direction.sends() || text.len() > MAX_CLIPBOARD_TEXT_BYTES {
            return;
        }
        self.local_text = Some(text);
        self.pending_messages
            .push_back(ClipboardMessage::SendInitiateCopy(vec![
                ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
            ]));
    }

    pub fn take_pending_messages(&mut self) -> Vec<ClipboardMessage> {
        self.pending_messages.drain(..).collect()
    }

    pub fn take_remote_text_update(&mut self) -> Option<String> {
        self.remote_text_update.take()
    }
}

impl Default for SessionClipboardBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ironrdp_core::AsAny for SessionClipboardBackend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl CliprdrBackend for SessionClipboardBackend {
    fn temporary_directory(&self) -> &str {
        &self.temporary_directory
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {}

    fn on_format_list_response(&mut self, ok: bool) {
        if self.bootstrap_pending {
            self.bootstrap_pending = false;
            if !ok {
                self.failure =
                    Some("Clipboard initialization was rejected. Reconnect the session.".into());
            }
            return;
        }
        if let Some(reply) = self.local_waiter.take() {
            let _ = reply.send(if ok {
                Ok(())
            } else {
                Err("Remote rejected the clipboard announcement. Retry resync.".into())
            });
        }
    }

    fn on_request_format_list(&mut self) {
        // MonitorReady requires an initial format list even with no data or
        // redirection disabled. Its ACK makes the upstream channel Ready.
        if self.bootstrap_pending {
            return;
        }
        self.bootstrap_pending = true;
        let formats = if self.direction.sends() && self.local_text.is_some() {
            vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
        } else {
            Vec::new()
        };
        self.pending_messages
            .push_back(ClipboardMessage::SendInitiateCopy(formats));
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        _capabilities: ClipboardGeneralCapabilityFlags,
    ) {
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        let preferred_format = available_formats
            .iter()
            .find(|format| format.id() == ClipboardFormatId::CF_UNICODETEXT)
            .map(|format| format.id())
            .or_else(|| {
                available_formats
                    .iter()
                    .find(|format| format.id() == ClipboardFormatId::CF_TEXT)
                    .map(|format| format.id())
            });

        self.remote_revision = self.remote_revision.wrapping_add(1);
        self.remote_format = preferred_format;
        self.request_remote();
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let response = match (
            request.format,
            self.local_text
                .as_deref()
                .filter(|_| self.direction.sends()),
        ) {
            (ClipboardFormatId::CF_UNICODETEXT, Some(text)) => {
                FormatDataResponse::new_unicode_string(text).into_owned()
            }
            (ClipboardFormatId::CF_TEXT, Some(text)) => {
                FormatDataResponse::new_string(text).into_owned()
            }
            _ => FormatDataResponse::new_error().into_owned(),
        };

        self.pending_messages
            .push_back(ClipboardMessage::SendFormatData(response));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let Some(format) = self.pending_remote_format.take() else {
            return;
        };
        if self.pending_revision != self.remote_revision {
            if let Some(reply) = self.remote_waiter.take() {
                let _ = reply.send(Err(
                    "Remote clipboard changed during transfer. Retry resync.".into(),
                ));
            }
            self.request_remote();
            return;
        }
        let expired = self
            .remote_waiter
            .as_ref()
            .is_some_and(|reply| reply.is_closed());
        let text = if response.is_error()
            || response.data().len() > MAX_CLIPBOARD_TEXT_BYTES * 2
            || expired
        {
            None
        } else {
            match Some(format) {
                Some(ClipboardFormatId::CF_UNICODETEXT) => response.to_unicode_string().ok(),
                Some(ClipboardFormatId::CF_TEXT) => response.to_string().ok(),
                _ => None,
            }
        };
        let result = if text.is_some() {
            Ok(())
        } else {
            Err(
                "Remote clipboard transfer failed or expired. Copy again and retry resync."
                    .to_string(),
            )
        };
        if let Some(reply) = self.remote_waiter.take() {
            let _ = reply.send(result.clone());
        }
        if let Err(error) = result {
            self.failure = Some(error);
        }

        if let Some(text) = text {
            self.remote_text_update = Some(text);
        }
    }

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}
}

/// Characterization tests for the plain-text clipboard backend seam
/// (task 3.3, Req 3.5). These pin the backend's message queueing and
/// format negotiation only — actual two-way clipboard synchronization with
/// a server is a real-server check (Req 12.9); mock frames prove nothing.
#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
    use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, FormatDataRequest};

    fn unicode_text_format() -> ClipboardFormat {
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)
    }

    #[tokio::test]
    async fn real_client_bootstraps_fresh_and_reconnected_channels_without_local_data() {
        use ironrdp_cliprdr::{
            CliprdrClient,
            pdu::{Capabilities, ClipboardPdu, ClipboardProtocolVersion, FormatListResponse},
        };
        use ironrdp_svc::SvcProcessor;
        for _connection in 0..2 {
            for direction in [
                ClipboardDirection::Bidirectional,
                ClipboardDirection::LocalToRemote,
                ClipboardDirection::RemoteToLocal,
                ClipboardDirection::Off,
            ] {
                let mut client = CliprdrClient::new(Box::new(
                    SessionClipboardBackend::with_direction(direction),
                ));
                client
                    .process(
                        &ironrdp_core::encode_vec(&ClipboardPdu::Capabilities(Capabilities::new(
                            ClipboardProtocolVersion::V2,
                            ClipboardGeneralCapabilityFlags::empty(),
                        )))
                        .unwrap(),
                    )
                    .unwrap();
                client
                    .process(&ironrdp_core::encode_vec(&ClipboardPdu::MonitorReady).unwrap())
                    .unwrap();
                assert!(!client.is_ready());
                let messages = client
                    .downcast_backend_mut::<SessionClipboardBackend>()
                    .unwrap()
                    .take_pending_messages();
                assert_eq!(
                    messages.len(),
                    1,
                    "MonitorReady must always produce an initial format list"
                );
                let ClipboardMessage::SendInitiateCopy(formats) = &messages[0] else {
                    panic!("expected initial formats")
                };
                assert!(
                    formats.is_empty(),
                    "fresh session has no permitted local clipboard data"
                );
                assert!(!Vec::from(client.initiate_copy(formats).unwrap()).is_empty());
                let (early_reply, early_result) = tokio::sync::oneshot::channel();
                client
                    .downcast_backend_mut::<SessionClipboardBackend>()
                    .unwrap()
                    .begin_local("too early".into(), early_reply);
                assert!(
                    early_result.await.unwrap().is_err(),
                    "resync must not share the bootstrap ACK"
                );
                let ack = ironrdp_core::encode_vec(&ClipboardPdu::FormatListResponse(
                    FormatListResponse::Ok,
                ))
                .unwrap();
                client.process(&ack).unwrap();
                assert!(client.is_ready());
                if direction.sends() {
                    let (tx, mut rx) = tokio::sync::oneshot::channel();
                    client
                        .downcast_backend_mut::<SessionClipboardBackend>()
                        .unwrap()
                        .begin_local("fresh after connect".into(), tx);
                    let messages = client
                        .downcast_backend_mut::<SessionClipboardBackend>()
                        .unwrap()
                        .take_pending_messages();
                    let ClipboardMessage::SendInitiateCopy(formats) = &messages[0] else {
                        panic!("expected resync formats")
                    };
                    client.initiate_copy(formats).unwrap();
                    assert!(
                        rx.try_recv().is_err(),
                        "bootstrap ACK must not resolve a later resync"
                    );
                    client.process(&ack).unwrap();
                    assert_eq!(rx.await.unwrap(), Ok(()));
                }
            }
        }
    }

    #[tokio::test]
    async fn recovery_waits_for_ack_and_rejects_overlapping_local_requests() {
        let mut backend = SessionClipboardBackend::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        backend.begin_local("fresh".into(), tx);
        assert!(rx.try_recv().is_err());
        let (tx2, rx2) = tokio::sync::oneshot::channel();
        backend.begin_local("second".into(), tx2);
        assert!(rx2.await.unwrap().is_err());
        backend.on_format_list_response(true);
        assert_eq!(rx.await.unwrap(), Ok(()));
    }

    #[tokio::test]
    async fn recovery_direction_and_stale_response_are_enforced() {
        use crate::store::connections::ClipboardDirection;
        let mut backend =
            SessionClipboardBackend::with_direction(ClipboardDirection::RemoteToLocal);
        let (tx, rx) = tokio::sync::oneshot::channel();
        backend.begin_local("private".into(), tx);
        assert!(rx.await.unwrap().is_err());
        assert!(backend.take_pending_messages().is_empty());
        backend.on_remote_copy(&[unicode_text_format()]);
        backend.take_pending_messages();
        backend.on_remote_copy(&[]);
        backend.on_format_data_response(FormatDataResponse::new_unicode_string("stale"));
        assert!(backend.take_remote_text_update().is_none());
    }

    #[tokio::test]
    async fn recovery_expired_requests_are_drained_before_retry() {
        let mut backend = SessionClipboardBackend::new();
        let (tx, rx) = tokio::sync::oneshot::channel();
        backend.begin_local("first".into(), tx);
        drop(rx);
        let (tx, rx) = tokio::sync::oneshot::channel();
        backend.begin_local("second".into(), tx);
        assert!(rx.await.unwrap().is_err());
        backend.on_format_list_response(true);
        let (tx, rx) = tokio::sync::oneshot::channel();
        backend.begin_local("retry".into(), tx);
        backend.on_format_list_response(false);
        assert!(rx.await.unwrap().is_err());
        backend.on_remote_copy(&[unicode_text_format()]);
        backend.on_format_data_response(FormatDataResponse::new_unicode_string("initial"));
        backend.take_remote_text_update();
        let (tx, rx) = tokio::sync::oneshot::channel();
        backend.begin_remote(tx);
        drop(rx);
        backend.on_format_data_response(FormatDataResponse::new_unicode_string("late"));
        assert!(backend.take_remote_text_update().is_none());
        let (tx, rx) = tokio::sync::oneshot::channel();
        backend.begin_remote(tx);
        backend.on_format_data_response(FormatDataResponse::new_unicode_string("fresh"));
        assert!(rx.await.unwrap().is_ok());
        assert_eq!(backend.take_remote_text_update().as_deref(), Some("fresh"));
    }

    #[test]
    fn recovery_off_and_size_limits_do_not_expose_content() {
        let mut backend = SessionClipboardBackend::with_direction(ClipboardDirection::Off);
        backend.set_local_text("private".into());
        backend.on_remote_copy(&[unicode_text_format()]);
        assert!(backend.take_pending_messages().is_empty());
        backend.on_format_data_request(FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT,
        });
        assert!(
            matches!(&backend.take_pending_messages()[0], ClipboardMessage::SendFormatData(data) if data.is_error())
        );
        let mut backend = SessionClipboardBackend::new();
        backend.set_local_text("x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1));
        assert!(backend.take_pending_messages().is_empty());
    }

    #[test]
    fn test_set_local_text_queues_unicode_text_copy_initiation() {
        let mut backend = SessionClipboardBackend::new();
        assert!(
            backend.take_pending_messages().is_empty(),
            "fresh backend queues nothing"
        );

        backend.set_local_text("hello drift".to_string());

        let messages = backend.take_pending_messages();
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            ClipboardMessage::SendInitiateCopy(formats) => {
                assert_eq!(formats.len(), 1);
                assert_eq!(formats[0].id(), ClipboardFormatId::CF_UNICODETEXT);
            }
            other => panic!("expected copy initiation, got {:?}", other),
        }
        // The queue drains completely.
        assert!(backend.take_pending_messages().is_empty());
    }

    #[test]
    fn test_initial_format_list_includes_empty_and_permitted_text_without_duplicate_bootstrap() {
        for text in [None, Some(""), Some("text")] {
            let mut backend = SessionClipboardBackend::new();
            if let Some(text) = text {
                backend.set_local_text(text.into());
                backend.take_pending_messages();
            }
            backend.on_request_format_list();
            backend.on_request_format_list();
            let messages = backend.take_pending_messages();
            assert_eq!(messages.len(), 1);
            assert!(
                matches!(&messages[0], ClipboardMessage::SendInitiateCopy(formats) if formats.len() == usize::from(text.is_some()))
            );
            backend.on_format_list_response(true);
        }
    }

    #[test]
    fn test_remote_copy_prefers_unicode_text_over_ansi() {
        let mut backend = SessionClipboardBackend::new();
        let formats = [
            ClipboardFormat::new(ClipboardFormatId::CF_TEXT),
            ClipboardFormat::new(ClipboardFormatId::CF_BITMAP),
            unicode_text_format(),
        ];
        backend.on_remote_copy(&formats);

        let messages = backend.take_pending_messages();
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            ClipboardMessage::SendInitiatePaste(format) => {
                assert_eq!(*format, ClipboardFormatId::CF_UNICODETEXT);
            }
            other => panic!("expected paste initiation, got {:?}", other),
        }
    }

    #[test]
    fn test_remote_copy_falls_back_to_ansi_and_ignores_non_text() {
        let mut backend = SessionClipboardBackend::new();

        // ANSI text is used when unicode text is not offered.
        backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_TEXT)]);
        let messages = backend.take_pending_messages();
        assert!(matches!(
            &messages[0],
            ClipboardMessage::SendInitiatePaste(format)
                if *format == ClipboardFormatId::CF_TEXT
        ));

        // Without any text format there is nothing to paste.
        backend.on_remote_copy(&[
            ClipboardFormat::new(ClipboardFormatId::CF_BITMAP),
            ClipboardFormat::new(ClipboardFormatId::CF_HDROP),
        ]);
        assert!(
            backend.take_pending_messages().is_empty(),
            "non-text remote copies are ignored"
        );
    }

    #[test]
    fn test_format_data_request_serves_local_text_for_both_text_formats() {
        let mut backend = SessionClipboardBackend::new();
        backend.set_local_text("drift".to_string());
        assert_eq!(backend.take_pending_messages().len(), 1);

        backend.on_format_data_request(FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT,
        });
        backend.on_format_data_request(FormatDataRequest {
            format: ClipboardFormatId::CF_TEXT,
        });

        let messages = backend.take_pending_messages();
        assert_eq!(messages.len(), 2);
        match &messages[0] {
            ClipboardMessage::SendFormatData(response) => {
                assert!(!response.is_error());
                assert_eq!(response.to_unicode_string().expect("unicode text"), "drift");
            }
            other => panic!("expected format data, got {:?}", other),
        }
        match &messages[1] {
            ClipboardMessage::SendFormatData(response) => {
                assert!(!response.is_error());
                assert_eq!(response.to_string().expect("ansi text").as_str(), "drift");
            }
            other => panic!("expected format data, got {:?}", other),
        }
    }

    #[test]
    fn test_format_data_request_without_local_text_answers_error() {
        let mut backend = SessionClipboardBackend::new();
        backend.on_format_data_request(FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT,
        });

        let messages = backend.take_pending_messages();
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            ClipboardMessage::SendFormatData(response) => {
                assert!(
                    response.is_error(),
                    "no local text must answer with an error"
                );
            }
            other => panic!("expected format data, got {:?}", other),
        }
    }

    #[test]
    fn test_format_data_response_publishes_remote_text_once() {
        let mut backend = SessionClipboardBackend::new();
        assert!(
            backend.take_remote_text_update().is_none(),
            "no remote text before any response"
        );

        backend.on_remote_copy(&[unicode_text_format()]);
        assert_eq!(backend.take_pending_messages().len(), 1);

        backend.on_format_data_response(FormatDataResponse::new_unicode_string("remote text"));
        assert_eq!(
            backend.take_remote_text_update(),
            Some("remote text".to_string())
        );
        // The update is consumed exactly once.
        assert_eq!(backend.take_remote_text_update(), None);
    }

    #[test]
    fn test_format_data_response_error_and_unsolicited_are_ignored_but_empty_clears() {
        let mut backend = SessionClipboardBackend::new();

        // Error responses never publish and clear the pending format.
        backend.on_remote_copy(&[unicode_text_format()]);
        assert_eq!(backend.take_pending_messages().len(), 1);
        backend.on_format_data_response(FormatDataResponse::new_error());
        assert_eq!(backend.take_remote_text_update(), None);

        // Empty text is a valid clipboard replacement; never retain stale local text.
        backend.on_remote_copy(&[unicode_text_format()]);
        assert_eq!(backend.take_pending_messages().len(), 1);
        backend.on_format_data_response(FormatDataResponse::new_unicode_string(""));
        assert_eq!(backend.take_remote_text_update(), Some(String::new()));

        // Unsolicited responses (no pending remote copy) are ignored.
        backend.on_format_data_response(FormatDataResponse::new_unicode_string("stray"));
        assert_eq!(backend.take_remote_text_update(), None);
    }
}
