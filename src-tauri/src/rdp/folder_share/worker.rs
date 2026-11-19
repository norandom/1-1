//! The share's file I/O runs off the session loop (rdp-folder-sharing task 5.1).
//!
//! RDP file transfers used to make typing and the pointer slow, because the
//! client did disk work on the thread that also handles input and frames.
//! [`WorkerBackend`] is the backend the RDPDR channel sees. It admits each
//! drive request against fixed limits and queues it for one worker thread,
//! which owns the real backend (and the bound root, through `keep_alive`).
//! The session loop never waits for the disk: completions come back through
//! [`RdpdrBackend::poll_deferred_messages`] in bounded batches, and `wake`
//! tells the loop when some are ready.
//!
//! Every admitted request gets exactly one reply carrying its device and
//! completion IDs. A request over the limits, a duplicate completion ID or a
//! stopped worker gets an immediate error reply of the right shape. Dropping
//! the backend ends the channel's generation: the worker stops, its handles
//! close, and replies still in flight go nowhere.

use super::status::ShareStatusCell;
use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::Rdpdr;
use ironrdp_rdpdr::backend::RdpdrBackend;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    AnyIoCtlCode, ClientDriveLockControlResponse, ClientDriveNotifyChangeDirectoryResponse,
    ClientDriveQueryDirectoryResponse, ClientDriveQueryInformationResponse,
    ClientDriveQuerySecurityResponse, ClientDriveQueryVolumeInformationResponse,
    ClientDriveSetInformationResponse, ClientDriveSetSecurityResponse, DecodedDeviceControlRequest,
    DeviceCloseResponse, DeviceControlRequest, DeviceControlResponse, DeviceCreateResponse,
    DeviceFlushBuffersResponse, DeviceIoRequest, DeviceIoResponse, DeviceReadResponse,
    DeviceWriteResponse, Information, NtStatus, PrinterIoRequest, ServerDeviceAnnounceResponse,
    ServerDriveIoRequest,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::pdu::esp::PortIoRequest;
use ironrdp_svc::SvcMessage;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, mpsc},
    thread,
    time::Duration,
};

/// How often the worker asks the backend for deferred completions (byte-range
/// locks, change notifications) while no request arrives.
const DEFERRED_POLL: Duration = Duration::from_millis(20);
/// How long a setup call (attach, detach, reset) may wait for the worker.
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Bounds on what one session's share may hold at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Requests admitted and not yet answered.
    pub(crate) requests: usize,
    /// Write payload bytes admitted and not yet answered.
    pub(crate) queued_bytes: usize,
    /// Replies handed to the session loop per poll; the rest wait for the next.
    pub(crate) batch: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            requests: 64,
            queued_bytes: 16 * 1024 * 1024,
            batch: 16,
        }
    }
}

/// A request's identity from ingress to completion.
type Key = (u32, u32);

#[derive(Debug, Clone, Copy)]
struct Ticket {
    major_function: u32,
    bytes: usize,
}

/// Counters for status and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WorkerStats {
    pub(crate) outstanding: usize,
    pub(crate) peak_outstanding: usize,
    pub(crate) queued_bytes: usize,
    pub(crate) completed: u64,
    pub(crate) overloaded: u64,
}

#[derive(Debug, Default)]
struct Ledger {
    outstanding: HashMap<Key, Ticket>,
    stats: WorkerStats,
}

impl Ledger {
    fn settle(&mut self, key: Key) -> Option<Ticket> {
        let ticket = self.outstanding.remove(&key);
        if let Some(ticket) = ticket {
            self.stats.queued_bytes -= ticket.bytes;
            self.stats.completed += 1;
        }
        self.stats.outstanding = self.outstanding.len();
        ticket
    }
}

type Call = Box<dyn FnOnce(&mut dyn RdpdrBackend) -> PduResult<Vec<SvcMessage>> + Send>;

enum Job {
    Io(ServerDriveIoRequest),
    Control(DecodedDeviceControlRequest<AnyIoCtlCode>),
    Call(Call, mpsc::SyncSender<PduResult<Vec<SvcMessage>>>),
}

/// A reply with the IDs and status read from its header on the worker thread.
struct Done {
    key: Option<Key>,
    status: Option<u32>,
    message: SvcMessage,
}

pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

/// The RDPDR backend of one channel lifetime, with its I/O on a worker thread.
pub(crate) struct WorkerBackend {
    jobs: Option<mpsc::SyncSender<Job>>,
    replies: mpsc::Receiver<Done>,
    ledger: Arc<Mutex<Ledger>>,
    limits: Limits,
    wake: Wake,
    /// Signalled with `wake`, for an async session loop to await.
    ready: Arc<tokio::sync::Notify>,
    drive_security: bool,
    /// What each drive achieved, by device ID, for the session details.
    statuses: HashMap<u32, ShareStatusCell>,
    worker: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for WorkerBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerBackend")
            .field("limits", &self.limits)
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

ironrdp_core::impl_as_any!(WorkerBackend);

impl WorkerBackend {
    /// Start the worker. `keep_alive` (the bound root) lives exactly as long
    /// as the worker, so its guards are released when the worker ends.
    pub(crate) fn spawn(
        backend: Box<dyn RdpdrBackend>,
        keep_alive: impl Send + 'static,
        limits: Limits,
        wake: Wake,
    ) -> Self {
        let drive_security = backend.supports_drive_security();
        // Admission bounds the queue; the extra room is for setup calls.
        let (jobs, inbox) = mpsc::sync_channel(limits.requests + 4);
        let (outbox, replies) = mpsc::channel();
        let ready = Arc::new(tokio::sync::Notify::new());
        let signal = Arc::clone(&ready);
        let wake: Wake = Arc::new(move || {
            // A stored permit means a wake is never lost between polls.
            signal.notify_one();
            wake();
        });
        let worker_wake = Arc::clone(&wake);
        let worker = thread::Builder::new()
            .name("1-1 folder share".into())
            .spawn(move || run(backend, keep_alive, inbox, outbox, worker_wake))
            .expect("start the folder share worker");
        Self {
            jobs: Some(jobs),
            replies,
            ledger: Arc::default(),
            limits,
            wake,
            ready,
            drive_security,
            statuses: HashMap::new(),
            worker: Some(worker),
        }
    }

    /// Record each drive's protocol evidence in its status (task 5.3).
    pub(crate) fn with_statuses(mut self, statuses: HashMap<u32, ShareStatusCell>) -> Self {
        self.statuses = statuses;
        self
    }

    fn status(&self, device_id: u32) -> Option<&ShareStatusCell> {
        self.statuses.get(&device_id)
    }

    /// Notified whenever completions are waiting for `poll_deferred_messages`.
    pub(crate) fn ready(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.ready)
    }

    pub(crate) fn stats(&self) -> WorkerStats {
        self.ledger().stats
    }

    fn ledger(&self) -> MutexGuard<'_, Ledger> {
        self.ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Admit one request, or say which error answers it right away.
    fn admit(&self, io: &DeviceIoRequest, bytes: usize) -> Result<(), NtStatus> {
        let mut ledger = self.ledger();
        let key = (io.device_id, io.completion_id);
        if ledger.outstanding.contains_key(&key) {
            ledger.stats.overloaded += 1;
            return Err(NtStatus::INVALID_PARAMETER);
        }
        if ledger.outstanding.len() >= self.limits.requests
            || ledger.stats.queued_bytes + bytes > self.limits.queued_bytes
        {
            ledger.stats.overloaded += 1;
            return Err(NtStatus::INSUFFICIENT_RESOURCES);
        }
        let ticket = Ticket {
            major_function: u32::from(io.major_function),
            bytes,
        };
        ledger.outstanding.insert(key, ticket);
        ledger.stats.queued_bytes += bytes;
        ledger.stats.outstanding = ledger.outstanding.len();
        ledger.stats.peak_outstanding = ledger.stats.peak_outstanding.max(ledger.stats.outstanding);
        Ok(())
    }

    fn queue(&self, job: Job, io: &DeviceIoRequest) -> bool {
        let sent = self
            .jobs
            .as_ref()
            .is_some_and(|jobs| jobs.try_send(job).is_ok());
        if !sent {
            self.ledger().settle((io.device_id, io.completion_id));
        }
        sent
    }

    /// Run a setup call on the worker and wait for its result.
    fn call(&self, call: Call) -> PduResult<Vec<SvcMessage>> {
        let (reply, result) = mpsc::sync_channel(1);
        let queued = self
            .jobs
            .as_ref()
            .is_some_and(|jobs| jobs.send(Job::Call(call, reply)).is_ok());
        if !queued {
            return Err(ironrdp_pdu::pdu_other_err!(
                "the folder share worker stopped"
            ));
        }
        result
            .recv_timeout(SETUP_TIMEOUT)
            .map_err(|_| ironrdp_pdu::pdu_other_err!("the folder share worker did not answer"))?
    }
}

impl Drop for WorkerBackend {
    fn drop(&mut self) {
        for status in self.statuses.values() {
            status.ended();
        }
        // Requests still unanswered are interrupted; the remote program that
        // issued them owns cleanup and retries.
        let interrupted: Vec<String> = self
            .ledger()
            .outstanding
            .iter()
            .map(|((device, completion), ticket)| {
                format!(
                    "device {device} completion {completion} operation {:#x}",
                    ticket.major_function
                )
            })
            .collect();
        if !interrupted.is_empty() {
            log::info!(
                "Folder share ended with {} requests interrupted: {}",
                interrupted.len(),
                interrupted.join(", ")
            );
        }
        // Closing the queue ends the worker after its current request; joining
        // happens off this thread, which must not wait for the disk.
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let _ = thread::Builder::new()
                .name("1-1 folder share join".into())
                .spawn(move || {
                    if worker.join().is_err() {
                        log::warn!("The folder share worker panicked");
                    }
                });
        }
    }
}

fn run(
    mut backend: Box<dyn RdpdrBackend>,
    keep_alive: impl Send,
    inbox: mpsc::Receiver<Job>,
    outbox: mpsc::Sender<Done>,
    wake: Wake,
) {
    // Reading each reply's header here keeps that work off the session loop.
    let deliver = |messages: Vec<SvcMessage>| -> bool {
        for message in messages {
            let header = header_of(&message);
            let done = Done {
                key: header.map(|(device, completion, _)| (device, completion)),
                status: header.map(|(_, _, status)| status),
                message,
            };
            if outbox.send(done).is_err() {
                return false;
            }
        }
        true
    };
    loop {
        let job = match inbox.recv_timeout(DEFERRED_POLL) {
            Ok(job) => Some(job),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut delivered = false;
        if let Some(job) = job {
            let alive = match job {
                Job::Io(request) => {
                    // Built before the request moves: the reply if the backend fails.
                    let fallback = error_reply(&request, NtStatus::UNSUCCESSFUL);
                    let messages = backend.handle_drive_io_request(request).or_else(|error| {
                        log::warn!("Folder share request failed: {error}");
                        fallback.map(|message| vec![message])
                    });
                    delivered = true;
                    deliver(messages.unwrap_or_default())
                }
                Job::Control(request) => {
                    let header = request.request.header.clone();
                    let messages =
                        backend
                            .handle_drive_device_control(request)
                            .unwrap_or_else(|error| {
                                log::warn!("Folder share control request failed: {error}");
                                vec![control_error(&header, NtStatus::UNSUCCESSFUL)]
                            });
                    delivered = true;
                    deliver(messages)
                }
                Job::Call(call, reply) => {
                    let _ = reply.send(call(backend.as_mut()));
                    true
                }
            };
            if !alive {
                break;
            }
        }
        match backend.poll_deferred_messages() {
            Ok(messages) if !messages.is_empty() => {
                delivered = true;
                if !deliver(messages) {
                    break;
                }
            }
            Ok(_) => {}
            Err(error) => log::warn!("Folder share deferred completions failed: {error}"),
        }
        if delivered {
            wake();
        }
    }
    // The generation ended: close every handle, then release the guards.
    drop(backend);
    drop(keep_alive);
}

/// The device ID, completion ID and NTSTATUS of an encoded reply.
fn header_of(message: &SvcMessage) -> Option<(u32, u32, u32)> {
    let bytes = message.encode_unframed_pdu().ok()?;
    let word = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    // RDPDR header (component and packet ID), then DeviceId, CompletionId, IoStatus.
    Some((word(4)?, word(8)?, word(12)?))
}

impl RdpdrBackend for WorkerBackend {
    fn supports_drive_security(&self) -> bool {
        self.drive_security
    }

    fn reset(&mut self) -> PduResult<()> {
        log::info!("Folder share: the server started the drive channel");
        // The server forgets outstanding requests on a channel reset.
        let reset = self.call(Box::new(|backend| backend.reset().map(|()| Vec::new())));
        let mut ledger = self.ledger();
        ledger.outstanding.clear();
        ledger.stats.outstanding = 0;
        ledger.stats.queued_bytes = 0;
        reset.map(drop)
    }

    fn restore_drive(&mut self, device_id: u32) -> PduResult<()> {
        self.call(Box::new(move |backend| {
            backend.restore_drive(device_id).map(|()| Vec::new())
        }))
        .map(drop)
    }

    fn add_drive(&mut self, device_id: u32) -> PduResult<()> {
        self.call(Box::new(move |backend| {
            backend.add_drive(device_id).map(|()| Vec::new())
        }))
        .map(drop)
    }

    fn remove_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
        self.call(Box::new(move |backend| backend.remove_drive(device_id)))
    }

    fn handle_server_device_announce_response(
        &mut self,
        pdu: ServerDeviceAnnounceResponse,
    ) -> PduResult<()> {
        if let Some(status) = self.status(pdu.device_id) {
            status.answered(u32::from(pdu.result_code));
        }
        self.call(Box::new(move |backend| {
            backend
                .handle_server_device_announce_response(pdu)
                .map(|()| Vec::new())
        }))
        .map(drop)
    }

    fn handle_scard_call(
        &mut self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        // The share announces one drive and no smart card.
        Ok(Vec::new())
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        let io = io_request(&req).clone();
        let bytes = match &req {
            ServerDriveIoRequest::DeviceWriteRequest(write) => write.write_data.len(),
            _ => 0,
        };
        if let Err(status) = self.admit(&io, bytes) {
            if let Some(cell) = self.status(io.device_id) {
                cell.replied(Some(u32::from(io.major_function)), u32::from(status));
            }
            return Ok(vec![error_reply(&req, status)?]);
        }
        let refusal = error_reply(&req, NtStatus::INSUFFICIENT_RESOURCES)?;
        if self.queue(Job::Io(req), &io) {
            Ok(Vec::new())
        } else {
            Ok(vec![refusal])
        }
    }

    fn handle_drive_device_control(
        &mut self,
        req: DecodedDeviceControlRequest<AnyIoCtlCode>,
    ) -> PduResult<Vec<SvcMessage>> {
        let header = req.request.header.clone();
        if let Err(status) = self.admit(&header, 0) {
            if let Some(cell) = self.status(header.device_id) {
                cell.replied(Some(u32::from(header.major_function)), u32::from(status));
            }
            return Ok(vec![control_error(&header, status)]);
        }
        if self.queue(Job::Control(req), &header) {
            Ok(Vec::new())
        } else {
            Ok(vec![control_error(
                &header,
                NtStatus::INSUFFICIENT_RESOURCES,
            )])
        }
    }

    fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        let mut messages = Vec::new();
        while messages.len() < self.limits.batch {
            let Ok(done) = self.replies.try_recv() else {
                break;
            };
            let ticket = done.key.and_then(|key| self.ledger().settle(key));
            if let (Some(status), Some((device_id, _))) = (done.status, done.key)
                && let Some(cell) = self.status(device_id)
            {
                // The reply goes on the wire right after this poll.
                cell.replied(ticket.map(|t| t.major_function), status);
            }
            messages.push(done.message);
        }
        if messages.len() == self.limits.batch {
            // More may be waiting: come back after the next input and frames.
            (self.wake)();
        }
        Ok(messages)
    }

    fn handle_user_logged_on(&mut self, _rdpdr: &mut Rdpdr) -> PduResult<Vec<SvcMessage>> {
        // The channel announces its drives right after this call.
        for status in self.statuses.values() {
            status.offered();
        }
        Ok(Vec::new())
    }

    fn handle_printer_io_request(&mut self, _req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    fn handle_port_io_request(&mut self, _req: PortIoRequest) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    fn reject_printer_write(&mut self, _req: DeviceIoRequest) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}

/// The common I/O header of a drive request.
pub(crate) fn io_request(req: &ServerDriveIoRequest) -> &DeviceIoRequest {
    match req {
        ServerDriveIoRequest::ServerCreateDriveRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQueryInformationRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceCloseRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceControlRequest(r) => &r.header,
        ServerDriveIoRequest::DeviceReadRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceWriteRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceFlushBuffersRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveSetInformationRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveLockControlRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQuerySecurityRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveSetSecurityRequest(r) => &r.device_io_request,
    }
}

fn control_error(header: &DeviceIoRequest, status: NtStatus) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse {
        device_io_reply: DeviceIoResponse::new(header.clone(), status),
        output_buffer: None,
    }))
}

/// A reply with `status` in the shape the request's operation requires.
pub(crate) fn error_reply(req: &ServerDriveIoRequest, status: NtStatus) -> PduResult<SvcMessage> {
    let reply = || DeviceIoResponse::new(io_request(req).clone(), status);
    let encode = |e| ironrdp_pdu::encode_err!(e);
    let pdu = match req {
        ServerDriveIoRequest::ServerCreateDriveRequest(_) => {
            RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                device_io_reply: reply(),
                file_id: 0,
                information: Information::empty(),
            })
        }
        ServerDriveIoRequest::ServerDriveQueryInformationRequest(_) => {
            RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                device_io_response: reply(),
                buffer: None,
            })
        }
        ServerDriveIoRequest::DeviceCloseRequest(_) => {
            RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
                device_io_response: reply(),
            })
        }
        ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(_) => {
            RdpdrPdu::ClientDriveQueryDirectoryResponse(ClientDriveQueryDirectoryResponse {
                device_io_reply: reply(),
                buffer: None,
            })
        }
        ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(_) => {
            RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(
                ClientDriveNotifyChangeDirectoryResponse {
                    device_io_reply: reply(),
                    buffer: Vec::new(),
                },
            )
        }
        ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(_) => {
            RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                ClientDriveQueryVolumeInformationResponse {
                    device_io_reply: reply(),
                    buffer: None,
                },
            )
        }
        ServerDriveIoRequest::DeviceControlRequest(control) => {
            return Ok(control_error(&control.header, status));
        }
        ServerDriveIoRequest::DeviceReadRequest(_) => {
            RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                device_io_reply: reply(),
                read_data: Vec::new(),
            })
        }
        ServerDriveIoRequest::DeviceWriteRequest(_) => {
            RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                device_io_reply: reply(),
                length: 0,
            })
        }
        ServerDriveIoRequest::DeviceFlushBuffersRequest(_) => {
            RdpdrPdu::DeviceFlushBuffersResponse(DeviceFlushBuffersResponse {
                device_io_response: reply(),
            })
        }
        ServerDriveIoRequest::ServerDriveSetInformationRequest(set) => {
            RdpdrPdu::ClientDriveSetInformationResponse(
                ClientDriveSetInformationResponse::new(set, status).map_err(encode)?,
            )
        }
        ServerDriveIoRequest::ServerDriveLockControlRequest(_) => {
            RdpdrPdu::ClientDriveLockControlResponse(ClientDriveLockControlResponse {
                device_io_reply: reply(),
            })
        }
        ServerDriveIoRequest::ServerDriveQuerySecurityRequest(_) => {
            RdpdrPdu::ClientDriveQuerySecurityResponse(ClientDriveQuerySecurityResponse {
                device_io_response: reply(),
                security_descriptor: None,
            })
        }
        ServerDriveIoRequest::ServerDriveSetSecurityRequest(set) => {
            RdpdrPdu::ClientDriveSetSecurityResponse(
                ClientDriveSetSecurityResponse::new(set, status).map_err(encode)?,
            )
        }
    };
    Ok(SvcMessage::from(pdu))
}

#[cfg(test)]
mod tests;
