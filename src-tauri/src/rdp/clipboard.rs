use std::{collections::VecDeque, env};

use ironrdp_cliprdr::{
    backend::{ClipboardMessage, CliprdrBackend},
    pdu::{
        ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
        FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    },
};
use ironrdp_core::IntoOwned;

#[derive(Debug)]
pub struct SessionClipboardBackend {
    temporary_directory: String,
    pending_messages: VecDeque<ClipboardMessage>,
    local_text: Option<String>,
    remote_text_update: Option<String>,
    pending_remote_format: Option<ClipboardFormatId>,
}

impl SessionClipboardBackend {
    pub fn new() -> Self {
        Self {
            temporary_directory: env::temp_dir().to_string_lossy().into_owned(),
            pending_messages: VecDeque::new(),
            local_text: None,
            remote_text_update: None,
            pending_remote_format: None,
        }
    }

    pub fn set_local_text(&mut self, text: String) {
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

    fn on_request_format_list(&mut self) {
        if self
            .local_text
            .as_deref()
            .is_some_and(|text| !text.is_empty())
        {
            self.pending_messages
                .push_back(ClipboardMessage::SendInitiateCopy(vec![
                    ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
                ]));
        }
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

        if let Some(format) = preferred_format {
            self.pending_remote_format = Some(format);
            self.pending_messages
                .push_back(ClipboardMessage::SendInitiatePaste(format));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let response = match (request.format, self.local_text.as_deref()) {
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
        if response.is_error() {
            self.pending_remote_format = None;
            return;
        }

        let text = match self.pending_remote_format.take() {
            Some(ClipboardFormatId::CF_UNICODETEXT) => response.to_unicode_string().ok(),
            Some(ClipboardFormatId::CF_TEXT) => response.to_string().ok(),
            _ => None,
        };

        if let Some(text) = text.filter(|text| !text.is_empty()) {
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
    fn test_request_format_list_reannounces_only_nonempty_local_text() {
        // No local text at all: nothing to announce.
        let mut backend = SessionClipboardBackend::new();
        backend.on_request_format_list();
        assert!(backend.take_pending_messages().is_empty());

        // Empty-string local text: set_local_text queues one initiation, but
        // the format-list hook treats it as nothing to copy.
        backend.set_local_text(String::new());
        assert_eq!(backend.take_pending_messages().len(), 1);
        backend.on_request_format_list();
        assert!(
            backend.take_pending_messages().is_empty(),
            "empty local text is never re-announced"
        );

        // Nonempty local text is re-announced: set_local_text queues its own
        // initiation AND the format-list hook queues a second announcement.
        backend.set_local_text("drift".to_string());
        backend.on_request_format_list();
        let messages = backend.take_pending_messages();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|message| matches!(
            message,
            ClipboardMessage::SendInitiateCopy(formats)
                if formats.len() == 1 && formats[0].id() == ClipboardFormatId::CF_UNICODETEXT
        )));
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
    fn test_format_data_response_error_empty_and_unsolicited_are_not_published() {
        let mut backend = SessionClipboardBackend::new();

        // Error responses never publish and clear the pending format.
        backend.on_remote_copy(&[unicode_text_format()]);
        assert_eq!(backend.take_pending_messages().len(), 1);
        backend.on_format_data_response(FormatDataResponse::new_error());
        assert_eq!(backend.take_remote_text_update(), None);

        // Empty strings are filtered out.
        backend.on_remote_copy(&[unicode_text_format()]);
        assert_eq!(backend.take_pending_messages().len(), 1);
        backend.on_format_data_response(FormatDataResponse::new_unicode_string(""));
        assert_eq!(backend.take_remote_text_update(), None);

        // Unsolicited responses (no pending remote copy) are ignored.
        backend.on_format_data_response(FormatDataResponse::new_unicode_string("stray"));
        assert_eq!(backend.take_remote_text_update(), None);
    }
}
