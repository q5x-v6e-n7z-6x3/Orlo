//! Paths from outside the running Orlo: macOS's open-files events (`open -a Orlo f`, a drop
//! on the Dock icon) and a second `Orlo f` launch, which hands its paths to the first over a
//! Unix socket and exits, as `subl` does. One Orlo, so one writer of Session.json.

use std::collections::HashMap;
use std::future::Future;
use std::io::{self, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

/// `~/Library/Application Support/Orlo/orlo.sock`
pub fn socket_path() -> PathBuf {
    orlo_utils::paths::support_dir().join("orlo.sock")
}

/// Batches of paths waiting for the UI thread, and the task waiting for them. An empty batch
/// only brings Orlo to the front.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Queue>>);

#[derive(Default)]
struct Queue {
    batches: Vec<Delivery>,
    receipts: HashMap<String, Arc<Receipt>>,
    waker: Option<Waker>,
}

impl Queue {
    fn push(&mut self, delivery: Delivery) {
        self.batches.push(delivery);
        if let Some(w) = self.waker.take() {
            w.wake();
        }
    }
}

#[derive(Default)]
struct Receipt {
    accepted: AtomicBool,
    abandoned: AtomicBool,
    outcome: Mutex<Option<(Instant, Result<(), String>)>>,
    clients: AtomicUsize,
}

impl Receipt {
    fn finish(&self, result: Result<(), String>) {
        let mut outcome = self.outcome.lock().unwrap_or_else(|e| e.into_inner());
        if outcome.is_none() {
            *outcome = Some((Instant::now(), result));
        }
    }

    fn outcome(&self) -> Option<Result<(), String>> {
        self.outcome.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, r)| r.clone())
    }
}

pub struct Completion(Option<Arc<Receipt>>);

impl Completion {
    pub fn outcome(&self) -> Option<Result<(), String>> {
        self.0.as_ref().and_then(|r| r.outcome())
    }

    pub fn finish(&self, result: Result<(), String>) {
        if let Some(receipt) = &self.0 {
            receipt.finish(result);
        }
    }

    pub fn connected(&self) -> bool {
        self.0.as_ref().is_none_or(|r| r.clients.load(Ordering::Acquire) > 0)
    }
}

/// A request removed from the queue is not yet accepted. The UI acknowledges it only after
/// opening its paths; dropping it during shutdown leaves the client free to retry.
pub struct Delivery {
    pub paths: Vec<PathBuf>,
    pub request: Option<crate::cli::Request>,
    receipt: Option<Arc<Receipt>>,
}

impl Delivery {
    pub fn local(request: crate::cli::Request) -> Self {
        Self {
            paths: Vec::new(),
            request: Some(request),
            receipt: Some(Arc::new(Receipt { clients: AtomicUsize::new(1), ..Default::default() })),
        }
    }

    pub fn completion(&self) -> Completion {
        Completion(self.receipt.clone())
    }

    pub fn accept(self) {
        self.begin_wait().finish(Ok(()));
    }

    /// Actual opens have succeeded. Retrying this ID must never open a second stdin buffer.
    pub fn begin_wait(self) -> Completion {
        if let Some(receipt) = &self.receipt {
            receipt.accepted.store(true, Ordering::Release);
        }
        Completion(self.receipt.clone())
    }

    pub fn reject(self, error: String) {
        if let Some(receipt) = &self.receipt {
            receipt.finish(Err(error));
            receipt.accepted.store(true, Ordering::Release);
        } else {
            eprintln!("Orlo: {error}");
        }
    }
}

impl Drop for Delivery {
    fn drop(&mut self) {
        if let Some(receipt) = &self.receipt {
            if !receipt.accepted.load(Ordering::Acquire) {
                receipt.abandoned.store(true, Ordering::Release);
            }
        }
    }
}

impl Inbox {
    pub fn push(&self, paths: Vec<PathBuf>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(Delivery { paths, request: None, receipt: None });
    }

    /// Retries of one launch have one ID, even after a lost acknowledgement or a cancelled
    /// quit. Keep accepted IDs longer than the client's five-second retry window.
    fn request(&self, id: String, paths: Vec<PathBuf>) -> Arc<Receipt> {
        self.request_message(id, paths, None)
            .unwrap_or_else(|_| Arc::new(Receipt { abandoned: AtomicBool::new(true), ..Default::default() }))
    }

    fn request_message(
        &self,
        id: String,
        paths: Vec<PathBuf>,
        request: Option<crate::cli::Request>,
    ) -> Result<Arc<Receipt>, String> {
        let connected = request.is_some();
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        q.receipts.retain(|_, r| {
            !r.abandoned.load(Ordering::Acquire)
                && r.outcome
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .is_none_or(|(at, _)| at.elapsed() < Duration::from_secs(60))
        });
        if let Some(r) = q.receipts.get(&id) {
            if !r.abandoned.load(Ordering::Acquire) {
                if connected {
                    r.clients.fetch_add(1, Ordering::AcqRel);
                }
                return Ok(r.clone());
            }
        }
        if q.receipts.len() >= 1024
            || q.receipts.values().filter(|r| r.outcome().is_none()).count() >= 64
            || q.batches.len() >= 64
        {
            return Err("too many pending launches; close a waiting request and try again".into());
        }
        let receipt = Arc::new(Receipt { clients: AtomicUsize::new(usize::from(connected)), ..Default::default() });
        q.receipts.insert(id, receipt.clone());
        q.push(Delivery { paths, request, receipt: Some(receipt.clone()) });
        Ok(receipt)
    }

    pub fn next_delivery(&self) -> impl Future<Output = Delivery> + '_ {
        Next(self)
    }

    /// Signals and local notifications need no acknowledgement from a separate process.
    pub async fn next(&self) -> Vec<PathBuf> {
        let mut delivery = self.next_delivery().await;
        let paths = std::mem::take(&mut delivery.paths);
        delivery.accept();
        paths
    }
}

struct Next<'a>(&'a Inbox);

impl Future for Next<'_> {
    type Output = Delivery;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Delivery> {
        let mut q = self.0 .0.lock().unwrap_or_else(|e| e.into_inner());
        if q.batches.is_empty() {
            q.waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        Poll::Ready(q.batches.remove(0))
    }
}

/// Paths as absolute (the other process has its own working folder), each ended by a NUL,
/// the one byte a path cannot hold.
fn encode(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in paths {
        let p = std::path::absolute(p).unwrap_or_else(|_| p.clone());
        out.extend_from_slice(p.as_os_str().as_bytes());
        out.push(0);
    }
    out
}

fn decode(bytes: &[u8]) -> Vec<PathBuf> {
    bytes.split(|&b| b == 0).filter(|l| !l.is_empty()).map(|l| PathBuf::from(std::ffi::OsStr::from_bytes(l))).collect()
}

/// What a launch is.
pub enum Claim {
    /// The one Orlo: it holds `Orlo.lock` (kept open while it runs), or none could be taken.
    Primary(Option<std::fs::File>),
    /// Another Orlo acknowledged accepting the paths.
    Secondary,
    /// No confirmed delivery: the launch must report failure, not exit successfully.
    Failed(String),
}

/// One Orlo per user, decided by a lock on `Orlo.lock` beside `sock`: two launches at once
/// cannot both restore the session. A launch that loses hands `paths` to the winner, retrying
/// while the winner starts listening, for up to five seconds.
pub fn claim(sock: &Path, paths: &[PathBuf]) -> Claim {
    claim_message(sock, paths, None)
}

pub fn claim_request(sock: &Path, request: &crate::cli::Request) -> Claim {
    if let Err(error) = request.validate() {
        return Claim::Failed(error);
    }
    claim_message(sock, &[], Some(request))
}

fn claim_message(sock: &Path, paths: &[PathBuf], request: Option<&crate::cli::Request>) -> Claim {
    let lock = sock.with_file_name("Orlo.lock");
    if let Some(dir) = sock.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = match std::fs::File::options().create(true).truncate(false).write(true).open(&lock) {
        Ok(f) => f,
        Err(e) => return Claim::Failed(format!("cannot open {}: {e}", lock.display())),
    };
    match file.try_lock() {
        Ok(()) => return Claim::Primary(Some(file)),
        Err(std::fs::TryLockError::Error(e)) => {
            return Claim::Failed(format!("cannot lock {}: {e}", lock.display()));
        }
        Err(std::fs::TryLockError::WouldBlock) => {}
    }
    let id = request_id();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match request {
            Some(request) => match cli_exchange(sock, request, &id, deadline) {
                Ok(()) => return Claim::Secondary,
                Err(ClientError::Rejected(error)) => return Claim::Failed(error),
                Err(ClientError::Retry) => {}
            },
            None if hand_off_request(sock, paths, &id, deadline) => return Claim::Secondary,
            None => {}
        }
        // The quitting primary has let go: restore its session, then open our original args.
        if file.try_lock().is_ok() {
            return Claim::Primary(Some(file));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Claim::Failed(
        "the running Orlo did not acknowledge the files. It may be busy or an older \
        version; quit it and launch this version again. Delivery was not confirmed."
            .into(),
    )
}

const GREETING: &[u8] = b"ORLO-HANDOFF-2\n";
const REQUEST: &[u8] = b"ORLO-HANDOFF-2\0";
const ACK: &[u8] = b"OK\n";
const IO_WAIT: Duration = Duration::from_secs(1);
const MAX_REQUEST: usize = 1024 * 1024;
const CLI_PROBE: &[u8] = b"ORLO-CLI-1\0";
const CLI_GREETING: &[u8] = b"ORLO-CLI-1\n";
const MAX_CLI_FRAME: usize = 16 * 1024 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
struct CliFrame {
    id: String,
    request: crate::cli::Request,
}

enum ClientError {
    Retry,
    Rejected(String),
}

/// The fresh --wait launch starts a normal GUI child, then remains only as its client.
pub fn send_request(sock: &Path, request: &crate::cli::Request) -> Result<(), String> {
    let id = request_id();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match cli_exchange(sock, request, &id, deadline) {
            Ok(()) => return Ok(()),
            Err(ClientError::Rejected(error)) => return Err(error),
            Err(ClientError::Retry) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    Err("the running Orlo did not confirm delivery; quit it and launch this version again".into())
}

fn cli_exchange(sock: &Path, request: &crate::cli::Request, id: &str, deadline: Instant) -> Result<(), ClientError> {
    let mut accepted = false;
    let mut sent = false;
    let send = (|| -> io::Result<Result<(), String>> {
        let mut stream = connect_until(sock, deadline)?;
        let mut greeting = [0; GREETING.len()];
        read_exact_until(&mut stream, &mut greeting, deadline.min(Instant::now() + IO_WAIT))?;
        if greeting != GREETING {
            return Ok(Err(
                "the running Orlo uses an incompatible handoff protocol; quit it and launch this version again".into(),
            ));
        }
        write_all_until(&mut stream, &[REQUEST, CLI_PROBE].concat(), deadline.min(Instant::now() + IO_WAIT))?;
        let mut capability = [0; CLI_GREETING.len()];
        read_exact_until(&mut stream, &mut capability, deadline.min(Instant::now() + IO_WAIT * 2))?;
        if capability != CLI_GREETING {
            return Ok(Err(
                "the running Orlo does not support this command line; quit it and launch this version again".into(),
            ));
        }
        let frame = serde_json::to_vec(&CliFrame { id: id.into(), request: request.clone() })?;
        if frame.len() > MAX_CLI_FRAME {
            return Ok(Err("command-line request exceeds the 16 MiB frame limit".into()));
        }
        write_all_until(&mut stream, &(frame.len() as u32).to_be_bytes(), Instant::now() + IO_WAIT)?;
        write_all_until(&mut stream, &frame, Instant::now() + IO_WAIT)?;
        sent = true;
        loop {
            let line = read_line(&mut stream)?;
            match line.as_str() {
                "PENDING" | "WAIT" => {}
                "OK" => accepted = true,
                "DONE" if accepted => return Ok(Ok(())),
                error if error.starts_with("ERROR ") => return Ok(Err(error[6..].into())),
                _ => return Ok(Err("the running Orlo sent an invalid response".into())),
            }
        }
    })();
    match send {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(ClientError::Rejected(error)),
        Err(error) if accepted => Err(ClientError::Rejected(format!("lost connection to Orlo before the requested buffers closed: {error}"))),
        Err(error) if sent => Err(ClientError::Rejected(format!("lost connection to Orlo after sending the request; it may already be open, but completion was not confirmed: {error}"))),
        Err(_) => Err(ClientError::Retry),
    }
}

fn read_line(stream: &mut UnixStream) -> io::Result<String> {
    let deadline = Instant::now() + IO_WAIT * 3;
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        read_exact_until(stream, &mut byte, deadline)?;
        if byte[0] == b'\n' {
            return String::from_utf8(bytes).map_err(|e| io::Error::new(ErrorKind::InvalidData, e));
        }
        if bytes.len() == 8192 {
            return Err(ErrorKind::InvalidData.into());
        }
        bytes.push(byte[0]);
    }
}

fn request_id() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{}-{now}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

/// Success means the UI accepted the request, not merely that a socket buffered its bytes.
/// An older server sends no greeting: no paths are sent to it, and the caller reports failure.
#[cfg(any(test, feature = "test-support"))]
pub fn hand_off(sock: &Path, paths: &[PathBuf]) -> bool {
    let id = request_id();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if hand_off_request(sock, paths, &id, deadline) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn hand_off_request(sock: &Path, paths: &[PathBuf], id: &str, deadline: Instant) -> bool {
    let send = || -> io::Result<bool> {
        let mut s = connect_until(sock, deadline)?;
        let wait = IO_WAIT.min(deadline.saturating_duration_since(Instant::now()));
        if wait.is_zero() {
            return Ok(false);
        }
        s.set_read_timeout(Some(wait))?;
        s.set_write_timeout(Some(wait))?;
        let mut greeting = [0; GREETING.len()];
        read_exact_until(&mut s, &mut greeting, Instant::now() + wait)?;
        if greeting != GREETING {
            return Ok(false);
        }
        let mut bytes = REQUEST.to_vec();
        bytes.extend_from_slice(id.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&encode(paths));
        if bytes.len() > MAX_REQUEST {
            return Ok(false);
        }
        write_all_until(&mut s, &bytes, deadline.min(Instant::now() + IO_WAIT))?;
        s.shutdown(Shutdown::Write)?;
        let wait = IO_WAIT.min(deadline.saturating_duration_since(Instant::now()));
        if wait.is_zero() {
            return Ok(false);
        }
        s.set_read_timeout(Some(wait))?;
        let mut ack = [0; ACK.len()];
        read_exact_until(&mut s, &mut ack, Instant::now() + wait)?;
        Ok(ack == ACK)
    };
    send().unwrap_or(false)
}

/// UnixStream::connect has no timeout. A full accept queue must not hang a launch before
/// its read/write deadlines are even installed.
fn connect_until(path: &Path, deadline: Instant) -> io::Result<UnixStream> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: sockaddr_un is a plain C structure, all-zero is a valid initial representation.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= addr.sun_path.len() || bytes.contains(&0) {
        return Err(ErrorKind::InvalidInput.into());
    }
    addr.sun_family = libc::AF_UNIX as _;
    for (dst, &src) in addr.sun_path.iter_mut().zip(bytes) {
        *dst = src as _;
    }
    let len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1) as libc::socklen_t;
    #[cfg(target_os = "macos")]
    {
        addr.sun_len = len as _;
    }
    // SAFETY: socket has no pointer arguments; on success we immediately own its descriptor.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a fresh, valid stream socket, owned only by this value from now on.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    #[cfg(target_os = "macos")]
    {
        let one: libc::c_int = 1;
        // SAFETY: the option points to a live integer of the stated size on our socket.
        if unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_NOSIGPIPE,
                (&one as *const libc::c_int).cast(),
                std::mem::size_of_val(&one) as _,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    // SAFETY: fcntl operates on our valid descriptor and takes no pointer for this command.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    stream.set_nonblocking(true)?;
    // SAFETY: addr has the matching family and len, and remains live throughout connect.
    let connected = unsafe { libc::connect(fd, (&addr as *const libc::sockaddr_un).cast(), len) };
    if connected < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(error);
        }
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            if wait.is_zero() {
                return Err(ErrorKind::TimedOut.into());
            }
            let mut poll = libc::pollfd { fd: stream.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
            let millis = wait.as_millis().clamp(1, i32::MAX as u128) as i32;
            // SAFETY: poll points to one valid pollfd for the entire call.
            match unsafe { libc::poll(&mut poll, 1, millis) } {
                0 => return Err(ErrorKind::TimedOut.into()),
                n if n < 0 => {
                    let e = io::Error::last_os_error();
                    if e.kind() != ErrorKind::Interrupted {
                        return Err(e);
                    }
                }
                _ => {
                    if let Some(e) = stream.take_error()? {
                        return Err(e);
                    }
                    stream.peer_addr()?;
                    break;
                }
            }
        }
    }
    stream.set_nonblocking(false)?;
    Ok(stream)
}

fn read_exact_until(s: &mut UnixStream, mut bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        read_timeout_until(s, deadline)?;
        match s.read(bytes) {
            Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
            Ok(n) => bytes = &mut bytes[n..],
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Reads wait only for what is left of `deadline`; `TimedOut` once it has passed.
fn read_timeout_until(s: &UnixStream, deadline: Instant) -> io::Result<()> {
    let wait = deadline.saturating_duration_since(Instant::now());
    if wait.is_zero() {
        return Err(ErrorKind::TimedOut.into());
    }
    match s.set_read_timeout(Some(wait)) {
        // macOS may reject timeout changes after peer close while reply bytes remain; the
        // read reaches them, or EOF, without waiting.
        Err(e) if e.raw_os_error() != Some(libc::EINVAL) => Err(e),
        _ => Ok(()),
    }
}

fn write_all_until(s: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        let wait = deadline.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        s.set_write_timeout(Some(wait))?;
        match s.write(bytes) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// One bounded frame. A trickling client cannot reset its time budget with every byte.
fn read_request(s: &mut UnixStream) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + IO_WAIT;
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        read_timeout_until(s, deadline)?;
        let n = match s.read(&mut chunk) {
            Ok(n) => n,
            // A legacy client does not read our greeting. Its close can reset the stream
            // after its complete NUL-terminated paths arrived; version 2 never uses this.
            Err(e)
                if e.kind() == ErrorKind::ConnectionReset
                    && !bytes.starts_with(REQUEST)
                    && (bytes.is_empty() || bytes.last() == Some(&0)) =>
            {
                return Ok(bytes)
            }
            Err(e) => return Err(e),
        };
        if n == 0 {
            return Ok(bytes);
        }
        if bytes.len() + n > MAX_REQUEST {
            return Err(ErrorKind::InvalidData.into());
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.strip_prefix(REQUEST) == Some(CLI_PROBE) {
            return Ok(bytes);
        }
    }
}

/// The socket this Orlo bound, where its hand-offs go, and whether its listener was stopped
/// for a quit (`stop_listening`).
type Bound = Option<(PathBuf, Inbox, Arc<AtomicBool>)>;

#[cfg(not(any(test, feature = "test-support")))]
fn with_bound<R>(f: impl FnOnce(&mut Bound) -> R) -> R {
    static BOUND: Mutex<Bound> = Mutex::new(None);
    f(&mut BOUND.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Tests: each has its own (a window test's quit must not take another test's socket).
#[cfg(any(test, feature = "test-support"))]
fn with_bound<R>(f: impl FnOnce(&mut Bound) -> R) -> R {
    thread_local!(static BOUND: std::cell::RefCell<Bound> = const { std::cell::RefCell::new(None) });
    BOUND.with_borrow_mut(f)
}

/// Stop taking hand-offs (Orlo is quitting): the socket goes first, so a launch from now on
/// waits for the lock instead of handing its paths to an Orlo about to exit. Only the socket
/// this Orlo bound is removed, and its listener's thread ends.
pub fn stop_listening() {
    with_bound(|b| {
        if let Some((sock, _, stopped)) = b {
            if !stopped.swap(true, Ordering::Relaxed) {
                let _ = std::fs::remove_file(sock);
            }
        }
    })
}

/// Called only once quitting is committed; a cancelled quit keeps existing waits alive.
pub fn finish_waiters() {
    with_bound(|b| {
        if let Some((_, inbox, _)) = b {
            let queue = inbox.0.lock().unwrap_or_else(|e| e.into_inner());
            for receipt in queue.receipts.values() {
                receipt.finish(if receipt.accepted.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err("Orlo quit before opening the requested files".into())
                });
            }
        }
    });
}

/// Called on the UI thread in the same update that opens and acknowledges a delivery.
/// Quit stops this gate before capturing the session; a cancelled quit opens it again.
pub fn accepting() -> bool {
    with_bound(|b| b.as_ref().is_none_or(|(_, _, stopped)| !stopped.load(Ordering::Acquire)))
}

/// The quit was called off (its session could not be written): take hand-offs again.
pub fn resume_listening() {
    match with_bound(Option::take) {
        Some((sock, inbox, stopped)) if stopped.load(Ordering::Relaxed) => {
            listen(&sock, inbox);
        }
        other => with_bound(|b| *b = other),
    }
}

/// Take hand-offs on `sock` from now on, on a thread of their own. A socket file left by a
/// crashed Orlo is replaced; false when another Orlo holds it. A client gets a second to send
/// its paths, so a stuck one cannot hold up the next.
pub fn listen(sock: &Path, inbox: Inbox) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = sock.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if connect_until(sock, Instant::now() + IO_WAIT).is_ok() {
        return false;
    }
    let _ = std::fs::remove_file(sock);
    let Ok(listener) = UnixListener::bind(sock) else {
        return false;
    };
    if std::fs::set_permissions(sock, std::fs::Permissions::from_mode(0o600)).is_err()
        || listener.set_nonblocking(true).is_err()
    {
        return false;
    }
    let stopped = Arc::new(AtomicBool::new(false));
    let clients = Arc::new(AtomicUsize::new(0));
    with_bound(|b| *b = Some((sock.into(), inbox.clone(), stopped.clone())));
    std::thread::spawn(move || {
        while !stopped.load(Ordering::Acquire) {
            let s = match listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    use std::os::fd::AsRawFd;
                    let mut ready = libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
                    // SAFETY: ready points to one live pollfd. The timeout lets stop finish
                    // without a potentially blocking wake-up connection; arrivals wake at once.
                    unsafe {
                        libc::poll(&mut ready, 1, 100);
                    }
                    continue;
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return,
            };
            if clients.fetch_add(1, Ordering::AcqRel) >= 8 {
                clients.fetch_sub(1, Ordering::AcqRel);
                continue;
            }
            let (slot, inbox, stopped) = (Slot(Some(clients.clone())), inbox.clone(), stopped.clone());
            std::thread::spawn(move || serve_connection(s, &inbox, &stopped, slot));
        }
    });
    true
}

/// A connection's share of the listener's cap; an acknowledged --wait gives it back early.
struct Slot(Option<Arc<AtomicUsize>>);

impl Slot {
    fn release(&mut self) {
        if let Some(clients) = self.0.take() {
            clients.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.release();
    }
}

fn serve_connection(mut stream: UnixStream, inbox: &Inbox, stopped: &AtomicBool, mut slot: Slot) {
    // The support path is discoverable; only this user's processes may supply editor input.
    // SAFETY: geteuid takes no arguments and cannot fail.
    if verify_peer(&stream, unsafe { libc::geteuid() }).is_err() {
        return;
    }
    // BSD/macOS can inherit the listener's nonblocking flag on accepted sockets.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    // A legacy client can close without reading our greeting; its buffered paths still count.
    let _ = write_all_until(&mut stream, GREETING, Instant::now() + IO_WAIT);
    let Ok(bytes) = read_request(&mut stream) else {
        return;
    };
    if stopped.load(Ordering::Acquire) {
        return;
    }
    let Some(frame) = bytes.strip_prefix(REQUEST) else {
        inbox.push(decode(&bytes));
        return;
    };
    if frame == CLI_PROBE {
        if let Err(error) = serve_cli(&mut stream, inbox, stopped, &mut slot) {
            let error = error.to_string().replace(['\r', '\n'], " ");
            let error: String = error.chars().take(4096).collect();
            let _ = write_all_until(&mut stream, format!("ERROR {error}\n").as_bytes(), Instant::now() + IO_WAIT);
        }
        return;
    }
    let Some(end) = frame.iter().position(|&b| b == 0) else {
        return;
    };
    let Ok(id) = std::str::from_utf8(&frame[..end]) else {
        return;
    };
    if id.is_empty() || id.len() > 128 {
        return;
    }
    let receipt = inbox.request(id.into(), decode(&frame[end + 1..]));
    let deadline = Instant::now() + IO_WAIT;
    while !receipt.accepted.load(Ordering::Acquire)
        && !receipt.abandoned.load(Ordering::Acquire)
        && !stopped.load(Ordering::Acquire)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(2));
    }
    if receipt.accepted.load(Ordering::Acquire) {
        let _ = write_all_until(&mut stream, ACK, Instant::now() + IO_WAIT);
    }
}

fn verify_peer(stream: &UnixStream, expected: libc::uid_t) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let (mut user, mut group) = (0, 0);
    // SAFETY: the live socket and writable credential integers outlive the call.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut user, &mut group) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if user != expected {
        return Err(ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

fn serve_cli(stream: &mut UnixStream, inbox: &Inbox, stopped: &AtomicBool, slot: &mut Slot) -> io::Result<()> {
    use std::io::Error;
    write_all_until(stream, CLI_GREETING, Instant::now() + IO_WAIT)?;
    let mut size = [0; 4];
    let deadline = Instant::now() + IO_WAIT;
    read_exact_until(stream, &mut size, deadline)?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > MAX_CLI_FRAME {
        return Err(Error::new(ErrorKind::InvalidData, "request exceeds the 16 MiB frame limit"));
    }
    let mut bytes = vec![0; size];
    read_exact_until(stream, &mut bytes, deadline)?;
    let frame: CliFrame = serde_json::from_slice(&bytes)?;
    if frame.id.is_empty() || frame.id.len() > 128 {
        return Err(Error::new(ErrorKind::InvalidData, "invalid request ID"));
    }
    frame.request.validate().map_err(Error::other)?;
    let receipt = inbox.request_message(frame.id, Vec::new(), Some(frame.request)).map_err(Error::other)?;
    struct Connected<'a>(&'a Receipt);
    impl Drop for Connected<'_> {
        fn drop(&mut self) {
            self.0.clients.fetch_sub(1, Ordering::AcqRel);
        }
    }
    let _connected = Connected(&receipt);
    let mut ping = Instant::now();
    while !receipt.accepted.load(Ordering::Acquire) {
        if let Some(Err(error)) = receipt.outcome() {
            return Err(Error::other(error));
        }
        if receipt.abandoned.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
            if receipt.accepted.load(Ordering::Acquire) {
                break;
            }
            return Err(Error::other("Orlo stopped before accepting the requested files"));
        }
        if peer_closed(stream) {
            return Ok(());
        }
        if ping.elapsed() >= Duration::from_millis(250) {
            write_all_until(stream, b"PENDING\n", Instant::now() + IO_WAIT)?;
            ping = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    write_all_until(stream, ACK, Instant::now() + IO_WAIT)?;
    slot.release();
    loop {
        if let Some(result) = receipt.outcome() {
            result.map_err(Error::other)?;
            return write_all_until(stream, b"DONE\n", Instant::now() + IO_WAIT);
        }
        if peer_closed(stream) {
            return Ok(());
        }
        if ping.elapsed() >= Duration::from_millis(250) {
            write_all_until(stream, b"WAIT\n", Instant::now() + IO_WAIT)?;
            ping = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn peer_closed(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut byte = 0u8;
    // SAFETY: byte is writable for one byte; PEEK leaves any input untouched and never blocks.
    let n = unsafe {
        libc::recv(stream.as_raw_fd(), (&mut byte as *mut u8).cast(), 1, libc::MSG_PEEK | libc::MSG_DONTWAIT)
    };
    n == 0 || n < 0 && !matches!(io::Error::last_os_error().kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted)
}

/// A `file://` URL as macOS sends it (percent-encoded); other schemes are not files.
pub fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let local;
    let rest = if let Some(path) = rest.strip_prefix("localhost/") {
        local = format!("/{path}");
        local.as_str()
    } else {
        rest
    };
    if !rest.starts_with('/') || rest.contains(['?', '#']) {
        return None;
    }
    let (mut out, bytes) = (Vec::new(), rest.as_bytes());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (b'%', _, _) => return None,
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    if out.contains(&0) {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_capabilities_are_negotiated_without_an_unsafe_legacy_path() {
        let sock = fresh("orlo-cli-capabilities");
        assert!(listen(&sock, Inbox::default()));
        let mut client = UnixStream::connect(&sock).unwrap();
        client.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let mut greeting = [0; GREETING.len()];
        client.read_exact(&mut greeting).unwrap();
        assert_eq!(greeting, GREETING, "version 2 clients still recognize the server");
        client.write_all(&[REQUEST, b"ORLO-CLI-1\0"].concat()).unwrap();
        let mut capability = [0; 11];
        let result = client.read_exact(&mut capability);
        stop_listening();
        assert!(result.is_ok(), "the CLI capability probe was not acknowledged: {result:?}");
        assert_eq!(&capability, b"ORLO-CLI-1\n");
    }

    /// The delivery waiting in `inbox`, if any, taken as the UI's receiver takes it.
    fn poll_once(inbox: &Inbox) -> Option<Delivery> {
        let mut task = Context::from_waker(Waker::noop());
        match std::pin::pin!(inbox.next_delivery()).poll(&mut task) {
            Poll::Ready(delivery) => Some(delivery),
            Poll::Pending => None,
        }
    }

    /// A listener on a fresh socket named `name`, and the inbox it delivers to.
    fn listening(name: &str) -> (PathBuf, Inbox) {
        let (sock, inbox) = (fresh(name), Inbox::default());
        assert!(listen(&sock, inbox.clone()));
        (sock, inbox)
    }

    fn next_cli(inbox: &Inbox) -> Delivery {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(delivery) = poll_once(inbox) {
                return delivery;
            }
            assert!(Instant::now() < deadline, "CLI request never reached the UI");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_queued_cli_request_has_a_live_client_before_the_ui_takes_it() {
        let inbox = Inbox::default();
        inbox.request_message("registration-order".into(), Vec::new(), Some(crate::cli::Request::default())).unwrap();
        let delivery = next_cli(&inbox);
        assert!(
            delivery.completion().connected(),
            "a fast UI must not mistake a newly queued --wait for a disconnected client"
        );
        delivery.accept();
    }

    #[test]
    fn a_lost_ack_after_the_frame_is_a_confirmed_failure() {
        let sock = fresh("orlo-cli-lost-ack");
        let listener = UnixListener::bind(&sock).unwrap();
        listener.set_nonblocking(true).unwrap();
        let request =
            crate::cli::Request { wait: true, targets: vec![crate::cli::Target::Stdin("one input buffer".into())] };
        std::thread::scope(|scope| {
            let server = scope.spawn(|| {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(error) => panic!("test server did not receive the launch: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                write_all_until(&mut stream, GREETING, deadline).unwrap();
                let mut probe = vec![0; REQUEST.len() + CLI_PROBE.len()];
                read_exact_until(&mut stream, &mut probe, deadline).unwrap();
                assert_eq!(probe, [REQUEST, CLI_PROBE].concat());
                write_all_until(&mut stream, CLI_GREETING, deadline).unwrap();
                let mut size = [0; 4];
                read_exact_until(&mut stream, &mut size, deadline).unwrap();
                let size = u32::from_be_bytes(size) as usize;
                assert!(size <= MAX_CLI_FRAME);
                let mut frame = vec![0; size];
                read_exact_until(&mut stream, &mut frame, deadline).unwrap();
                serde_json::from_slice::<CliFrame>(&frame).unwrap()
                // Close without an acknowledgement after the full frame was received.
            });
            let result = cli_exchange(&sock, &request, "lost-before-ack", Instant::now() + Duration::from_secs(5));
            let received = server.join().unwrap();
            assert_eq!(received.request, request);
            match result {
                Err(ClientError::Rejected(error)) => assert!(error.contains("connection"), "useful failure: {error}"),
                Err(ClientError::Retry) => panic!(
                    "a lost acknowledgement after sending the frame must not retry or report premature wait completion"
                ),
                Ok(()) => panic!("a dropped connection cannot confirm a successful wait"),
            }
        });
    }

    struct CliCleanup(Inbox);

    impl Drop for CliCleanup {
        fn drop(&mut self) {
            stop_listening();
            let queue = self.0 .0.lock().unwrap_or_else(|e| e.into_inner());
            for receipt in queue.receipts.values() {
                receipt.finish(Err("test fixture ended before completion".into()));
            }
        }
    }

    #[test]
    fn wait_completion_does_not_hold_up_a_second_launch() {
        let (sock, inbox) = listening("orlo-cli-wait-following");
        let request =
            crate::cli::Request { wait: true, targets: vec![crate::cli::Target::Stdin("piped café".into())] };
        std::thread::scope(|scope| {
            let _cleanup = CliCleanup(inbox.clone());
            let (done, result) = std::sync::mpsc::channel();
            let (socket, requested) = (&sock, &request);
            let waiting = scope.spawn(move || done.send(send_request(socket, requested)).unwrap());
            let delivery = next_cli(&inbox);
            assert_eq!(delivery.request.as_ref(), Some(&request));
            let completion = delivery.begin_wait();
            assert!(result.try_recv().is_err(), "opening a buffer alone does not release --wait");
            let second = scope.spawn(|| send_request(&sock, &crate::cli::Request::default()));
            next_cli(&inbox).accept();
            let following = second.join().unwrap();
            assert!(following.is_ok(), "listener accepts while another client waits: {following:?}");
            assert!(result.try_recv().is_err(), "other launches do not release --wait");
            completion.finish(Ok(()));
            assert!(result.recv_timeout(Duration::from_secs(5)).unwrap().is_ok());
            waiting.join().unwrap();
        });
        stop_listening();
    }

    #[test]
    fn accepted_waits_do_not_use_up_the_connection_cap() {
        let sock = fresh("orlo-cli-many-waits");
        let inbox = Inbox::default();
        assert!(listen(&sock, inbox.clone()));
        let request = crate::cli::Request { wait: true, ..Default::default() };
        std::thread::scope(|scope| {
            let _cleanup = CliCleanup(inbox.clone());
            let (socket, requested) = (&sock, &request);
            let mut waits = Vec::new();
            for _ in 0..9 {
                let waiting = scope.spawn(move || send_request(socket, requested));
                waits.push((waiting, next_cli(&inbox).begin_wait()));
            }
            let second = scope.spawn(|| send_request(&sock, &crate::cli::Request::default()));
            next_cli(&inbox).accept();
            let following = second.join().unwrap();
            assert!(following.is_ok(), "open waits blocked a launch: {following:?}");
            for (waiting, completion) in waits {
                completion.finish(Ok(()));
                assert!(waiting.join().unwrap().is_ok());
            }
        });
        stop_listening();
    }

    #[test]
    fn non_utf8_paths_and_locations_cross_the_socket_without_loss() {
        use std::os::unix::ffi::OsStringExt;
        let (sock, inbox) = listening("orlo-cli-byte-path");
        let request = crate::cli::Request {
            wait: false,
            targets: vec![crate::cli::Target::Path {
                path: PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/orlo-cli-\xff\n.txt".to_vec())),
                position: Some(crate::cli::Position { line: 7, column: 3 }),
            }],
        };
        std::thread::scope(|scope| {
            let _cleanup = CliCleanup(inbox.clone());
            let client = scope.spawn(|| send_request(&sock, &request));
            let delivery = next_cli(&inbox);
            assert_eq!(delivery.request.as_ref(), Some(&request));
            delivery.accept();
            let result = client.join().unwrap();
            assert!(result.is_ok(), "byte-path client: {result:?}");
        });
        stop_listening();
    }

    #[test]
    fn a_cancelled_quit_keeps_accepted_waits_alive() {
        let (sock, inbox) = listening("orlo-cli-cancelled-quit");
        let request = crate::cli::Request { wait: true, ..Default::default() };
        std::thread::scope(|scope| {
            let _cleanup = CliCleanup(inbox.clone());
            let (done, result) = std::sync::mpsc::channel();
            let (socket, requested) = (&sock, &request);
            let waiting = scope.spawn(move || done.send(send_request(socket, requested)).unwrap());
            let completion = next_cli(&inbox).begin_wait();
            stop_listening();
            std::thread::sleep(Duration::from_millis(40));
            assert!(result.try_recv().is_err(), "a tentative quit is not a completed wait");
            resume_listening();
            let next = scope.spawn(|| send_request(&sock, &crate::cli::Request::default()));
            next_cli(&inbox).accept();
            let following = next.join().unwrap();
            assert!(following.is_ok(), "following launch after cancelled quit: {following:?}");
            completion.finish(Ok(()));
            assert!(result.recv_timeout(Duration::from_secs(5)).unwrap().is_ok());
            waiting.join().unwrap();
        });
        stop_listening();
    }

    #[test]
    fn committed_quit_releases_accepted_waits_and_rejections_reach_the_client() {
        let (sock, inbox) = listening("orlo-cli-quit-wait");
        std::thread::scope(|scope| {
            let _cleanup = CliCleanup(inbox.clone());
            let request = crate::cli::Request { wait: true, ..Default::default() };
            let requested = request.clone();
            let socket = &sock;
            let client = scope.spawn(move || send_request(socket, &requested));
            let _completion = next_cli(&inbox).begin_wait();
            finish_waiters();
            let result = client.join().unwrap();
            assert!(result.is_ok(), "quit client: {result:?}");
            let rejected = scope.spawn(move || send_request(socket, &request));
            next_cli(&inbox).reject("could not read requested file".into());
            assert!(rejected.join().unwrap().unwrap_err().contains("could not read requested file"));
        });
        stop_listening();
    }

    #[test]
    fn a_lost_wait_client_releases_its_connection_worker() {
        let (sock, inbox) = listening("orlo-cli-lost-client");
        let mut client = UnixStream::connect(&sock).unwrap();
        let mut greeting = [0; GREETING.len()];
        client.read_exact(&mut greeting).unwrap();
        client.write_all(&[REQUEST, CLI_PROBE].concat()).unwrap();
        let mut capability = [0; CLI_GREETING.len()];
        client.read_exact(&mut capability).unwrap();
        let frame = serde_json::to_vec(&CliFrame {
            id: "lost-client".into(),
            request: crate::cli::Request { wait: true, ..Default::default() },
        })
        .unwrap();
        client.write_all(&(frame.len() as u32).to_be_bytes()).unwrap();
        client.write_all(&frame).unwrap();
        let completion = next_cli(&inbox).begin_wait();
        assert!(completion.connected());
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(5);
        while completion.connected() {
            assert!(Instant::now() < deadline, "lost client retained a worker");
            std::thread::sleep(Duration::from_millis(5));
        }
        completion.finish(Ok(()));
        stop_listening();
    }

    #[test]
    fn a_second_launch_hands_its_paths_to_the_first() {
        let sock = std::env::temp_dir().join("orlo-handoff-test.sock");
        let inbox = Inbox::default();
        assert!(listen(&sock, inbox.clone()));
        assert!(!listen(&sock, Inbox::default()), "a second listener gives way");
        assert_eq!(first(&inbox), Some(Vec::new()), "the legacy empty probe activates");
        let sent = vec![PathBuf::from("/tmp/a b.txt"), PathBuf::from("rel.txt")];
        let got = deliver(&sock, &inbox, &sent);
        assert_eq!(got[0], PathBuf::from("/tmp/a b.txt"));
        assert_eq!(got[1], std::env::current_dir().unwrap().join("rel.txt"));
        assert!(deliver(&sock, &inbox, &[]).is_empty());
        assert!(!hand_off(&std::env::temp_dir().join("orlo-nobody.sock"), &sent));
    }

    #[test]
    fn a_stopped_listener_ends_its_thread_and_a_resumed_one_takes_hand_offs() {
        let (sock, inbox) = listening("orlo-handoff-stop");
        // held by the test, the bound socket and the listener's thread
        assert_eq!(Arc::strong_count(&inbox.0), 3);
        stop_listening();
        assert!(!sock.exists(), "the socket goes first");
        // the minute only stops a hang
        let start = Instant::now();
        while Arc::strong_count(&inbox.0) > 2 {
            assert!(start.elapsed() < Duration::from_secs(60), "still waiting to accept");
            std::thread::sleep(Duration::from_millis(5));
        }
        resume_listening();
        assert_eq!(deliver(&sock, &inbox, &[PathBuf::from("/tmp/after")]), [PathBuf::from("/tmp/after")]);
        stop_listening();
    }

    fn fresh(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("orlo.sock")
    }

    /// The first batch `inbox` gets; `None` after half a minute (a loaded machine is slow,
    /// not stuck).
    fn first(inbox: &Inbox) -> Option<Vec<PathBuf>> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            if let Some(mut delivery) = poll_once(inbox) {
                let paths = std::mem::take(&mut delivery.paths);
                delivery.accept();
                return Some(paths);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    fn deliver(sock: &Path, inbox: &Inbox, paths: &[PathBuf]) -> Vec<PathBuf> {
        std::thread::scope(|scope| {
            let (done, result) = std::sync::mpsc::channel();
            let client = scope.spawn(move || done.send(hand_off(sock, paths)).unwrap());
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut got = Vec::new();
            let mut accept_ready = || {
                while let Some(mut delivery) = poll_once(inbox) {
                    let paths = std::mem::take(&mut delivery.paths);
                    if delivery.receipt.is_some() {
                        got.push(paths);
                    } else {
                        assert!(paths.is_empty(), "unexpected legacy file request: {paths:?}");
                    }
                    delivery.accept();
                }
            };
            let acknowledged = loop {
                // Model main's continuous UI receiver: a legacy activation is not the file
                // request, and must not stop the receiver before the file can be accepted.
                accept_ready();
                match result.try_recv() {
                    Ok(result) => break result,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => panic!("client stopped"),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
                assert!(Instant::now() < deadline, "handoff did not finish");
                std::thread::sleep(Duration::from_millis(2));
            };
            client.join().unwrap();
            accept_ready();
            assert!(acknowledged, "acknowledged by the UI");
            assert_eq!(got, [decode(&encode(paths))], "the exact request, once only");
            got.remove(0)
        })
    }

    #[test]
    fn an_activation_before_a_file_does_not_stop_the_ui_receiving_files() {
        let (sock, inbox) = listening("orlo-activation-before-file");
        inbox.push(Vec::new());
        assert_eq!(deliver(&sock, &inbox, &[PathBuf::from("/x")]), [PathBuf::from("/x")]);
        assert!(inbox.0.lock().unwrap().batches.is_empty());
        stop_listening();
    }

    #[test]
    fn a_request_is_not_accepted_until_the_ui_says_so_and_retries_are_deduplicated() {
        let inbox = Inbox::default();
        let paths = vec![PathBuf::from("/tmp/draft")];
        let receipt = inbox.request("launch-1".into(), paths.clone());
        let retry = inbox.request("launch-1".into(), paths.clone());
        assert!(Arc::ptr_eq(&receipt, &retry));
        assert_eq!(inbox.0.lock().unwrap().batches.len(), 1);
        let delivery = poll_once(&inbox).expect("queued");
        assert_eq!(delivery.paths, paths);
        assert!(!receipt.accepted.load(Ordering::Acquire), "dequeue is not acceptance");
        delivery.accept();
        assert!(receipt.accepted.load(Ordering::Acquire));
        // The acknowledgement got lost. A retry is acknowledged without opening the paths again.
        let retry = inbox.request("launch-1".into(), paths);
        assert!(retry.accepted.load(Ordering::Acquire));
        assert!(inbox.0.lock().unwrap().batches.is_empty());
    }

    #[test]
    fn a_request_dropped_during_shutdown_is_not_acknowledged() {
        let inbox = Inbox::default();
        let receipt = inbox.request("launch-2".into(), vec![PathBuf::from("/tmp/draft")]);
        drop(poll_once(&inbox).expect("queued"));
        assert!(!receipt.accepted.load(Ordering::Acquire));
        // Quit was cancelled. The same request can now be delivered and acknowledged.
        let retry = inbox.request("launch-2".into(), vec![PathBuf::from("/tmp/draft")]);
        assert!(!Arc::ptr_eq(&receipt, &retry));
        assert_eq!(first(&inbox), Some(vec![PathBuf::from("/tmp/draft")]));
        assert!(retry.accepted.load(Ordering::Acquire));
    }

    #[test]
    fn an_unaccepted_socket_write_is_not_a_successful_handoff() {
        let sock = fresh("orlo-handoff-no-accept");
        let _listener = UnixListener::bind(&sock).unwrap();
        assert!(!hand_off(&sock, &[PathBuf::from("/tmp/draft")]));
    }

    #[test]
    fn of_two_launches_at_once_one_is_primary_and_the_other_hands_off() {
        for round in 0..20 {
            let sock = fresh(&format!("orlo-race-{round}"));
            let start = std::sync::Barrier::new(2);
            let launch = |name: &str| {
                start.wait();
                match claim(&sock, &[PathBuf::from(name)]) {
                    Claim::Primary(lock) => {
                        let inbox = Inbox::default();
                        assert!(listen(&sock, inbox.clone()));
                        let got = first(&inbox).expect("the UI takes the other launch's paths");
                        Some((lock, got))
                    }
                    Claim::Secondary => None,
                    Claim::Failed(e) => panic!("{e}"),
                }
            };
            let (a, b) = std::thread::scope(|s| {
                let a = s.spawn(|| launch("/a"));
                let b = s.spawn(|| launch("/b"));
                (a.join().unwrap(), b.join().unwrap())
            });
            let primary: Vec<_> = [a, b].into_iter().flatten().collect();
            assert_eq!(primary.len(), 1, "round {round}");
            let got = primary[0].1.clone();
            assert!(got == [PathBuf::from("/a")] || got == [PathBuf::from("/b")]);
        }
    }

    #[test]
    fn a_launch_waiting_for_a_quitting_orlo_becomes_the_orlo_when_it_lets_go() {
        let sock = fresh("orlo-quitting-primary");
        // the other Orlo holds the lock but no longer listens (it is quitting)
        let lock = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(sock.with_file_name("Orlo.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let let_go = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(lock);
        });
        let started = Instant::now();
        let claimed = claim(&sock, &[PathBuf::from("/a")]);
        let_go.join().unwrap();
        assert!(matches!(claimed, Claim::Primary(Some(_))), "it takes over");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_quitting_orlo_takes_no_hand_offs_until_it_stays() {
        let (sock, inbox) = listening("orlo-stop-listening");
        stop_listening();
        assert!(!hand_off(&sock, &[PathBuf::from("/x")]), "the socket is gone");
        resume_listening();
        assert_eq!(deliver(&sock, &inbox, &[PathBuf::from("/x")]), [PathBuf::from("/x")]);
    }

    #[test]
    fn a_client_that_sends_nothing_does_not_hold_up_the_next() {
        let (sock, inbox) = listening("orlo-stuck-client");
        let stuck = UnixStream::connect(&sock).unwrap();
        assert_eq!(deliver(&sock, &inbox, &[PathBuf::from("/x")]), [PathBuf::from("/x")]);
        drop(stuck);
    }

    #[test]
    fn shutdown_before_ui_acceptance_is_not_success_and_resume_keeps_one_request() {
        let (sock, inbox) = listening("orlo-handoff-quit-before-ack");
        let paths = vec![PathBuf::from("/tmp/draft")];
        let send = || hand_off_request(&sock, &paths, "quit-race", Instant::now() + Duration::from_secs(5));
        std::thread::scope(|scope| {
            let client = scope.spawn(send);
            let deadline = Instant::now() + Duration::from_secs(5);
            while inbox.0.lock().unwrap().batches.is_empty() {
                assert!(Instant::now() < deadline, "request never queued");
                std::thread::sleep(Duration::from_millis(2));
            }
            stop_listening();
            assert!(!accepting());
            assert!(!client.join().unwrap(), "a buffered request is not acceptance");
        });
        resume_listening();
        assert!(accepting());
        std::thread::scope(|scope| {
            let client = scope.spawn(send);
            assert_eq!(first(&inbox), Some(paths.clone()));
            assert!(client.join().unwrap());
            assert!(inbox.0.lock().unwrap().batches.is_empty(), "retry did not duplicate");
        });
        stop_listening();
    }

    #[test]
    fn legacy_clients_can_send_paths_or_activate_without_reading_the_greeting() {
        let (sock, inbox) = listening("orlo-handoff-legacy-client");
        let paths = vec![PathBuf::from("/tmp/legacy")];
        let mut client = UnixStream::connect(&sock).unwrap();
        client.write_all(&encode(&paths)).unwrap();
        drop(client);
        assert_eq!(first(&inbox), Some(paths));
        // A legacy no-argument launch, including a close with the greeting unread.
        let client = UnixStream::connect(&sock).unwrap();
        drop(client);
        assert_eq!(first(&inbox), Some(Vec::new()));
        stop_listening();
    }

    #[test]
    fn an_old_server_gets_no_paths_without_a_version_greeting() {
        let sock = fresh("orlo-handoff-legacy-server");
        let listener = UnixListener::bind(&sock).unwrap();
        std::thread::scope(|scope| {
            let old = scope.spawn(|| {
                let (mut s, _) = listener.accept().unwrap();
                s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut bytes = Vec::new();
                s.read_to_end(&mut bytes).unwrap();
                bytes
            });
            assert!(!hand_off_request(
                &sock,
                &[PathBuf::from("/tmp/draft")],
                "old-server",
                Instant::now() + Duration::from_secs(3)
            ));
            assert!(old.join().unwrap().is_empty(), "no unsupported request sent");
        });
    }

    #[test]
    fn a_path_with_a_line_break_in_its_name_arrives_whole() {
        let paths = vec![PathBuf::from("/tmp/a\nb.txt"), PathBuf::from("/c")];
        assert_eq!(decode(&encode(&paths)), paths);
    }

    #[test]
    fn file_urls_decode_to_paths() {
        assert_eq!(
            file_url_path("file:///Users/me/My%20Notes/caf%C3%A9.txt"),
            Some(PathBuf::from("/Users/me/My Notes/café.txt"))
        );
        assert_eq!(file_url_path("file://localhost/tmp/x"), Some(PathBuf::from("/tmp/x")));
        assert_eq!(file_url_path("https://example.com"), None);
    }

    #[test]
    fn file_urls_reject_nonlocal_or_malformed_paths() {
        for url in [
            "file://server/tmp/x",
            "file://localhost-evil/tmp/x",
            "file://relative",
            "file:///tmp/%GG",
            "file:///tmp/%",
            "file:///tmp/%00",
            "file:///tmp/x?query",
            "file:///tmp/x#fragment",
        ] {
            assert_eq!(file_url_path(url), None, "{url}");
        }
    }

    #[test]
    fn handoff_socket_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let sock = fresh("orlo-handoff-private");
        assert!(listen(&sock, Inbox::default()));
        let mode = std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        stop_listening();
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn peer_identity_comes_from_the_kernel() {
        let (peer, _) = UnixStream::pair().unwrap();
        // SAFETY: geteuid has no arguments and cannot fail.
        let user = unsafe { libc::geteuid() };
        verify_peer(&peer, user).unwrap();
        assert_eq!(verify_peer(&peer, user.wrapping_add(1)).unwrap_err().kind(), ErrorKind::PermissionDenied);
    }
}
