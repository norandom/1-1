# RDP folder sharing research

## Summary

Full discovery, 2026-11-18. The released IronRDP Windows backend supports a single subdirectory and read-only mode. The app must bind the folder and peer identities, keep synchronous disk operations off the session actor, and measure actual server behavior. Findings below come from the repository and published package source. Dependency compilation, Windows integration and live-host results remain open.

## Research log

| Question | Finding and implication |
|---|---|
| Which dependency? | Both 0.9.0 crates require Rust 1.94. Their core 0.3, pdu 0.11 and svc 0.10 dependencies match the app's locked 0.3.1, 0.11.0 and 0.10.0 semver lines. Verify the resolved lock and SVC types at implementation. The native single-drive factory takes a normalized drive-letter path and read-only mode. No whole-drive discovery is necessary. [Published native factory](https://github.com/Devolutions/IronRDP/blob/0bd400f97fe01ef417e94aab846af84474ad88fb/crates/ironrdp-rdpdr-native/src/windows/factory.rs) · [Native manifest](https://github.com/Devolutions/IronRDP/blob/0bd400f97fe01ef417e94aab846af84474ad88fb/crates/ironrdp-rdpdr-native/Cargo.toml) |
| Does the root keep the same identity? | The released native source stores an NT path in RootDirectory. Each traversal reopens it by name. Descendant opens use directory handles and reject reparse traversal. Another ordinary directory can take the selected path and pass path validation. Bind a live authorized directory handle or protect and verify the complete parent chain. Exercise read-only opens, writes, metadata and control requests on the precise native build. [Windows handles](https://github.com/Devolutions/IronRDP/blob/0bd400f97fe01ef417e94aab846af84474ad88fb/crates/ironrdp-rdpdr-native/src/windows/handles.rs) · [File access](https://github.com/Devolutions/IronRDP/blob/0bd400f97fe01ef417e94aab846af84474ad88fb/crates/ironrdp-rdpdr-native/src/windows/file.rs) |
| Can authentication precede registration? | In the pinned async 0.12 and connector 0.12 sources, connect_begin stops at the security upgrade. The later MCS/GCC exchange in connect_finalize reads the static-channel set. Retain the connector across TLS and register after verification. The current app verifier accepts both the certificate and the handshake signatures. Simply hashing that certificate cannot establish key possession. Validate the TLS 1.2/1.3 server signature and bind that connection's authenticated identity to the explicit grant. [App verifier](../../../src-tauri/src/rdp/client.rs) · [rustls verifier contract](https://docs.rs/rustls/latest/rustls/client/danger/trait.ServerCertVerifier.html) |
| How do overlapping requests work? | Device/file IDs, completion IDs and offsets identify operations. The protocol crate accepts decoded requests and drains deferred responses. The native Windows backend executes ordinary reads and writes synchronously. Certain locks and notifications finish later. Track completions in both paths and use a bounded worker so input does not wait for disk. [Portable backend](https://github.com/Devolutions/IronRDP/blob/b13befa0db86f48f523a0b2b007e399f9811e0ec/crates/ironrdp-rdpdr/src/backend/mod.rs) · [Windows dispatcher](https://github.com/Devolutions/IronRDP/blob/0bd400f97fe01ef417e94aab846af84474ad88fb/crates/ironrdp-rdpdr-native/src/windows/backend.rs) · [Microsoft filesystem I/O](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/1a8715b1-3afc-4bd7-8ec2-9625a9ce9610) |
| Which state can the client prove? | IronRDP tracks pending, accepted and rejected devices. File access requires a successful operation. Only the host can confirm its mount path. xrdp exposes configurable FUSE mount settings. An embedded IronRDP server can optionally register a device-redirection component. Neither source establishes what the lab deployments have enabled. [Device lifecycle](https://github.com/Devolutions/IronRDP/blob/b13befa0db86f48f523a0b2b007e399f9811e0ec/crates/ironrdp-rdpdr/src/lib.rs) · [xrdp configuration](https://github.com/neutrinolabs/xrdp/blob/devel/sesman/sesman.ini.in) · [IronRDP server registration](https://github.com/Devolutions/IronRDP/blob/abfc97b299f04622cb7860f344a35bbea9bd4361/crates/ironrdp-server/src/server.rs#L3676-L3679) |

## Architecture options and decisions

| Option | Decision |
|---|---|
| Adapt the native backend at the grant and session boundaries | Selected. Reuse typed protocol processing, Windows file operations, read-only mode and error mapping. Prove root identity and deferred handling before enabling the device. |
| Implement a second filesystem backend or remap paths as strings | Adds maintenance across Windows operations. String remapping loses the handle-level scope guarantee. |
| Segment ordinary copies transparently | The remote application and its redirector choose the offsets. The client cannot assume a requested range implies a copy of the rest of the file. Evaluate an explicit bounded range copier only if the baseline shows an opportunity. |
| Add workspace deltas to the mount | Repeated workspace copies need source/destination state and an explicit sync operation. The sibling workspace delta-sync spec owns that behavior. |

## Risks and verification

- Prove that local replacement, aliases, rename destinations and descendant races cannot broaden the grant. If the released native API cannot bind the root safely, adapt upstream or keep the device disabled.
- Exercise interleaved I/O, deferred notifications, overload and disconnect. Compare input-to-echo timings and whole-session bytes with a matched no-copy baseline.
- Identify the actual Linux server deployments and observe their mounts with ordinary shell tools. The name xfreerdp identifies a client. Record lack of evidence as inconclusive.
