# Research and design decisions

## Summary

The saved credential test can stop before desktop setup. The point at which the client can report success depends on the negotiated NLA mode. Manual display pause can reuse the session’s existing suppression writer after separating deliberate remote input from local focus and resize events.

## Research log

### What does the pinned connector prove?

The lockfile resolves `ironrdp-async` and `ironrdp-connector` to 0.12.0 and `sspi` to 0.23.0. `connect_finalize` continues from NLA into desktop establishment. The public `perform_credssp` function stops earlier and returns `ConnectorResult<()>`. In the connector’s `CredsspSequence::handle_process_result`, plain HYBRID reaches `Finished` when the client produces its final token. HYBRID_EX transitions to `EarlyUserAuthResult` and reads a server reply. The spec also distinguishes a denial of access from a rejection of the password. For a verified HYBRID_EX exchange, success is positive evidence. For plain HYBRID local completion and early authorization denial, the credential result is inconclusive. [Pinned lockfile](../../../src-tauri/Cargo.lock) · [Async driver](https://docs.rs/crate/ironrdp-async/0.12.0/source/src/connector.rs) · [Connector state machine](https://docs.rs/crate/ironrdp-connector/0.12.0/source/src/credssp.rs) · [Microsoft early authorization PDU](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/d0e560a3-25cb-4563-8bdc-6c4cc625bbfc)

A server can include an NTSTATUS in a CredSSP TSRequest. The pinned decoder exposes `TsRequest.error_code`, and the sequence returns the decoded server request. Classify allowlisted credential-specific failures at that point. The convenience driver consumes this evidence internally. `ConnectorErrorKind::AccessDenied` from the early authorization PDU has different semantics. Preserve the source of the failure in the probe. Avoid classifying generic `AccessDenied`, locally generated errors or a closed socket as a rejected password. [SSPI TSRequest](https://docs.rs/crate/sspi/0.23.0/source/src/credssp/ts_request/mod.rs) · [Microsoft TSRequest](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cssp/6aac4dea-08ef-47a6-8747-22ea7f6d8685)

The existing client’s `NoCertVerifier` accepts peer certificates and handshake signatures. Extracting its public key cannot identify the configured host. Both designs expect an endpoint-bound SPKI result from verified TLS on the same connection. Credential tests consume an existing trust decision or normal certificate trust. Creating a directory grant remains a separate operator action. [Current client](../../../src-tauri/src/rdp/client.rs) · [Folder-sharing requirements](../rdp-folder-sharing/requirements.md)

### Where should manual display pause run?

`IdlePreview` already has a 300-second idle timer, a 60-second period and a two-second preview window. Its automatic trial returns to live on quiet or expensive desktops, and auto-off masks capability. The session currently treats every mouse command, resize and `ResumeDisplay` as activity. The view sends `ResumeDisplay` on focus. The actor already owns the socket, frame completion signals and clipboard channels. Extending that actor enables a single ordered Allow/input write path. [Preview policy](../../../src-tauri/src/rdp/idle_preview.rs) · [Session loop](../../../src-tauri/src/rdp/session.rs) · [Focus handling](../../../src/components/session/SessionView.tsx)

Microsoft specifies that the server stops or resumes graphics after processing Suppress Output. There is no per-request acknowledgment in this exchange. Continued completed desktop paints during a stable suppression interval can indicate that a host ignores the request. An isolated in-flight update or an expensive allow-window repaint cannot establish that. [Microsoft Suppress Output processing](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/d1ba009d-47b9-440a-b7b2-105a4b063c36)

## Architecture pattern evaluation and design decisions

| Option | Decision and reason |
| --- | --- |
| Reuse full desktop connect for credential testing | Use only X.224, verified TLS and CredSSP. Full connect crosses the requested boundary and may attach channels or start a session. |
| Interpret any completed TLS or NLA step as green | Require attributable server evidence. Some compatible hosts will legitimately stay gray. |
| Frontend timer or local repaint throttling for pause | Send negotiated suppression through the session actor. Local paint throttling alone would still receive normal graphics traffic. |
| Persist manual pause on the host | Keep it session-local. Reconnect applies the operator’s saved automatic preference. |

## Risks and validation

Use a fixture with fragmented CredSSP messages to verify error provenance and a tester-controlled Windows host to record real protocol evidence. Check failure paths against certificate mismatch, generic policy denial and no positive acknowledgment. Restrict diagnostic traces because the pinned connector has debug events containing decoded requests. Use virtual time for timing and combine Windows real-host resize, keyboard and clipboard tests with measured bytes on an active desktop. A forced refresh on a quiet desktop can cost more than ordinary deltas. The existing measurements cover automatic trials only. [Existing bandwidth evidence](../windows-clearcodec-royalts-e2e/bandwidth-research.md)
