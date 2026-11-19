//! Task 5.1: requests leave the calling thread at once, every request gets
//! one reply with its own IDs, limits answer instead of blocking, and
//! teardown releases the worker's state.
use super::*;
use ironrdp_rdpdr::pdu::efs::{
    DeviceReadRequest, DeviceWriteRequest, LockOperation, MajorFunction, MinorFunction,
    ServerDriveLockControlRequest,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

const SUCCESS: u32 = 0;
const INSUFFICIENT_RESOURCES: u32 = 0xC000_009A;
const INVALID_PARAMETER: u32 = 0xC000_000D;
const UNSUCCESSFUL: u32 = 0xC000_0001;

/// How the fake disk behaves.
#[derive(Default)]
struct Script {
    /// While set, every request waits until it is cleared.
    stalled: AtomicBool,
    /// Requests the backend has started.
    started: AtomicUsize,
    /// Lock requests answer only from `poll_deferred_messages`.
    deferred: Mutex<Vec<SvcMessage>>,
    /// Requests that fail inside the backend.
    fail: AtomicBool,
    dropped: AtomicBool,
    /// The thread each setup call ran on.
    setup_threads: Mutex<Vec<String>>,
}

#[derive(Debug)]
struct FakeDisk(Arc<Script>);
ironrdp_core::impl_as_any!(FakeDisk);

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Script")
    }
}

impl Drop for FakeDisk {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}

impl RdpdrBackend for FakeDisk {
    fn handle_server_device_announce_response(
        &mut self,
        _pdu: ServerDeviceAnnounceResponse,
    ) -> PduResult<()> {
        Ok(())
    }
    fn handle_scard_call(
        &mut self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        unreachable!()
    }
    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        self.0.started.fetch_add(1, Ordering::SeqCst);
        while self.0.stalled.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(2));
        }
        if self.0.fail.load(Ordering::SeqCst) {
            return Err(ironrdp_pdu::pdu_other_err!("disk error"));
        }
        let reply = |data: Vec<u8>, io: &DeviceIoRequest| {
            SvcMessage::from(RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                device_io_reply: DeviceIoResponse::new(io.clone(), NtStatus::SUCCESS),
                read_data: data,
            }))
        };
        Ok(match &req {
            // A read returns its file ID and offset, so a reply proves its request.
            ServerDriveIoRequest::DeviceReadRequest(read) => {
                let mut data = read.device_io_request.file_id.to_le_bytes().to_vec();
                data.extend(read.offset.to_le_bytes());
                vec![reply(data, &read.device_io_request)]
            }
            ServerDriveIoRequest::ServerDriveLockControlRequest(_) => {
                let answer = error_reply(&req, NtStatus::SUCCESS)?;
                self.0.deferred.lock().unwrap().push(answer);
                Vec::new()
            }
            _ => vec![error_reply(&req, NtStatus::SUCCESS)?],
        })
    }
    fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        Ok(std::mem::take(&mut *self.0.deferred.lock().unwrap()))
    }
    fn add_drive(&mut self, _device_id: u32) -> PduResult<()> {
        self.record();
        Ok(())
    }
    fn remove_drive(&mut self, _device_id: u32) -> PduResult<Vec<SvcMessage>> {
        self.record();
        Ok(Vec::new())
    }
}

impl FakeDisk {
    fn record(&self) {
        let name = thread::current().name().unwrap_or_default().to_owned();
        self.0.setup_threads.lock().unwrap().push(name);
    }
}

struct Harness {
    backend: WorkerBackend,
    script: Arc<Script>,
    wakes: Arc<AtomicUsize>,
    released: Arc<AtomicBool>,
}

/// Dropped with the worker, like the bound root.
struct Guard(Arc<AtomicBool>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn harness(limits: Limits) -> Harness {
    let script = Arc::new(Script::default());
    let wakes = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicBool::new(false));
    let counter = Arc::clone(&wakes);
    let backend = WorkerBackend::spawn(
        Box::new(FakeDisk(Arc::clone(&script))),
        Guard(Arc::clone(&released)),
        limits,
        Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    );
    Harness {
        backend,
        script,
        wakes,
        released,
    }
}

fn io(major_function: MajorFunction, file_id: u32, completion_id: u32) -> DeviceIoRequest {
    DeviceIoRequest {
        device_id: 1,
        file_id,
        completion_id,
        major_function,
        minor_function: MinorFunction::from(0),
    }
}

fn read(file_id: u32, completion_id: u32, offset: u64) -> ServerDriveIoRequest {
    ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
        device_io_request: io(MajorFunction::Read, file_id, completion_id),
        length: 16,
        offset,
    })
}

fn write(file_id: u32, completion_id: u32, bytes: usize) -> ServerDriveIoRequest {
    ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
        device_io_request: io(MajorFunction::Write, file_id, completion_id),
        offset: 0,
        write_data: vec![7; bytes],
    })
}

fn lock(file_id: u32, completion_id: u32) -> ServerDriveIoRequest {
    ServerDriveIoRequest::ServerDriveLockControlRequest(ServerDriveLockControlRequest {
        device_io_request: io(MajorFunction::LockControl, file_id, completion_id),
        operation: LockOperation::Shared,
        wait: false,
        locks: Vec::new(),
    })
}

/// Completion ID, status and body of an encoded reply.
fn decode(message: &SvcMessage) -> (u32, u32, Vec<u8>) {
    let bytes = message.encode_unframed_pdu().unwrap();
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    (word(8), word(12), bytes[16..].to_vec())
}

impl Harness {
    /// Replies until `count` arrived, polling as the session loop would.
    fn replies(&mut self, count: usize) -> Vec<(u32, u32, Vec<u8>)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut replies = Vec::new();
        while replies.len() < count && Instant::now() < deadline {
            let batch = self.backend.poll_deferred_messages().unwrap();
            assert!(batch.len() <= self.backend.limits.batch, "bounded batches");
            replies.extend(batch.iter().map(decode));
            thread::sleep(Duration::from_millis(1));
        }
        replies
    }
}

#[test]
fn interleaved_requests_each_get_one_reply_with_their_own_ids() {
    let mut h = harness(Limits::default());
    let mut expected = Vec::new();
    for completion in 0..40u32 {
        let file = completion % 3;
        let request = if completion % 4 == 3 {
            write(file, completion, 512)
        } else {
            expected.push((completion, file, u64::from(completion) * 4096));
            read(file, completion, u64::from(completion) * 4096)
        };
        let started = Instant::now();
        assert!(
            h.backend
                .handle_drive_io_request(request)
                .unwrap()
                .is_empty()
        );
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "never waits for the disk"
        );
    }
    let mut replies = h.replies(40);
    assert_eq!(replies.len(), 40);
    replies.sort_by_key(|reply| reply.0);
    let ids: Vec<u32> = replies.iter().map(|reply| reply.0).collect();
    assert_eq!(ids, (0..40).collect::<Vec<_>>(), "one reply per request");
    for (completion, file, offset) in expected {
        let (_, status, body) = &replies[completion as usize];
        assert_eq!(*status, SUCCESS);
        // DR_READ_RSP: Length, then the data the fake disk read.
        let data = &body[4..];
        assert_eq!(&data[..4], &file.to_le_bytes());
        assert_eq!(&data[4..12], &offset.to_le_bytes());
    }
    let stats = h.backend.stats();
    assert_eq!(
        (stats.outstanding, stats.queued_bytes, stats.completed),
        (0, 0, 40)
    );
    assert!(
        h.wakes.load(Ordering::SeqCst) > 0,
        "the session loop was woken"
    );
}

#[test]
fn a_stalled_disk_never_blocks_the_caller_and_limits_answer_at_once() {
    let limits = Limits {
        requests: 4,
        queued_bytes: 4096,
        batch: 16,
    };
    let mut h = harness(limits);
    h.script.stalled.store(true, Ordering::SeqCst);
    for completion in 0..4 {
        let started = Instant::now();
        assert!(
            h.backend
                .handle_drive_io_request(read(1, completion, 0))
                .unwrap()
                .is_empty()
        );
        assert!(started.elapsed() < Duration::from_millis(250));
    }
    // Full: the fifth request is answered right away, in its own shape.
    let started = Instant::now();
    let refused = h.backend.handle_drive_io_request(write(1, 4, 16)).unwrap();
    assert!(started.elapsed() < Duration::from_millis(250));
    let (completion, status, body) = decode(&refused[0]);
    assert_eq!((completion, status), (4, INSUFFICIENT_RESOURCES));
    assert_eq!(body, [0, 0, 0, 0, 0], "DR_WRITE_RSP: Length 0 and padding");
    assert_eq!(h.backend.stats().peak_outstanding, 4);

    h.script.stalled.store(false, Ordering::SeqCst);
    assert_eq!(h.replies(4).len(), 4);
    // Buffered write bytes are bounded too.
    let too_big = h
        .backend
        .handle_drive_io_request(write(1, 5, 5000))
        .unwrap();
    assert_eq!(decode(&too_big[0]).1, INSUFFICIENT_RESOURCES);
    assert!(
        h.backend
            .handle_drive_io_request(write(1, 6, 4096))
            .unwrap()
            .is_empty()
    );
    assert_eq!(h.replies(1)[0].0, 6);
    assert_eq!(h.backend.stats().overloaded, 2);
}

#[test]
fn a_duplicate_completion_id_is_refused_while_the_first_is_outstanding() {
    let mut h = harness(Limits::default());
    h.script.stalled.store(true, Ordering::SeqCst);
    assert!(
        h.backend
            .handle_drive_io_request(read(1, 9, 0))
            .unwrap()
            .is_empty()
    );
    let duplicate = h.backend.handle_drive_io_request(read(2, 9, 0)).unwrap();
    assert_eq!(decode(&duplicate[0]).1, INVALID_PARAMETER);
    h.script.stalled.store(false, Ordering::SeqCst);
    assert_eq!(h.replies(1).len(), 1);
}

#[test]
fn deferred_completions_reach_the_session_with_their_ids() {
    let mut h = harness(Limits::default());
    assert!(
        h.backend
            .handle_drive_io_request(lock(4, 21))
            .unwrap()
            .is_empty()
    );
    assert!(
        h.backend
            .handle_drive_io_request(read(4, 22, 8))
            .unwrap()
            .is_empty()
    );
    let mut replies = h.replies(2);
    replies.sort_by_key(|reply| reply.0);
    assert_eq!(
        replies.iter().map(|r| (r.0, r.1)).collect::<Vec<_>>(),
        [(21, SUCCESS), (22, SUCCESS)]
    );
    assert_eq!(
        h.backend.stats().outstanding,
        0,
        "the deferred reply settled its request"
    );
}

#[test]
fn a_failing_disk_still_answers_every_request() {
    let mut h = harness(Limits::default());
    h.script.fail.store(true, Ordering::SeqCst);
    h.backend.handle_drive_io_request(write(1, 30, 64)).unwrap();
    h.backend.handle_drive_io_request(read(1, 31, 0)).unwrap();
    let mut replies = h.replies(2);
    replies.sort_by_key(|reply| reply.0);
    assert_eq!((replies[0].0, replies[0].1), (30, UNSUCCESSFUL));
    assert_eq!(replies[0].2, [0, 0, 0, 0, 0], "a write's shape");
    assert_eq!((replies[1].0, replies[1].1), (31, UNSUCCESSFUL));
    assert_eq!(replies[1].2, [0, 0, 0, 0], "a read's shape: no data");
}

#[test]
fn replies_leave_in_bounded_batches_and_the_loop_is_woken_for_the_rest() {
    let mut h = harness(Limits {
        requests: 64,
        queued_bytes: 1 << 20,
        batch: 4,
    });
    for completion in 0..10 {
        h.backend
            .handle_drive_io_request(read(1, completion, 0))
            .unwrap();
    }
    // Let the worker answer everything before the session polls.
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.script.started.load(Ordering::SeqCst) < 10 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    thread::sleep(Duration::from_millis(50));
    let woken = h.wakes.load(Ordering::SeqCst);
    let first = h.backend.poll_deferred_messages().unwrap();
    assert_eq!(first.len(), 4, "one batch");
    assert!(
        h.wakes.load(Ordering::SeqCst) > woken,
        "woken again for the rest"
    );
    assert_eq!(h.replies(6).len(), 6);
}
#[test]
fn teardown_stops_the_worker_and_releases_handles_and_guards() {
    let h = harness(Limits::default());
    h.script.stalled.store(true, Ordering::SeqCst);
    let Harness {
        mut backend,
        script,
        released,
        ..
    } = h;
    backend.handle_drive_io_request(write(1, 40, 64)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while script.started.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    let started = Instant::now();
    drop(backend);
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "teardown never waits for the disk"
    );
    // The stalled request finishes, its late reply goes nowhere, the worker ends.
    script.stalled.store(false, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !(released.load(Ordering::SeqCst) && script.dropped.load(Ordering::SeqCst))
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        script.dropped.load(Ordering::SeqCst),
        "the backend's handles are closed"
    );
    assert!(
        released.load(Ordering::SeqCst),
        "the root guards are released"
    );
}

#[test]
fn setup_calls_run_on_the_worker() {
    let mut h = harness(Limits::default());
    h.backend.add_drive(1).unwrap();
    h.backend.restore_drive(1).unwrap();
    assert!(h.backend.remove_drive(1).unwrap().is_empty());
    h.backend.reset().unwrap();
    assert_eq!(h.backend.stats().outstanding, 0);
    let threads = h.script.setup_threads.lock().unwrap().clone();
    assert_eq!(
        threads,
        ["1-1 folder share", "1-1 folder share"],
        "attach and detach ran on the worker"
    );
}
