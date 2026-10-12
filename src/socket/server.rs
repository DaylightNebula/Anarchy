//! [`WorldSocket`], serving a [`World`] to other programs over an OS socket.

use std::{fmt, io::{BufRead, BufReader, Read, Write}, net::{TcpListener, TcpStream}, str::FromStr, sync::{Arc, atomic::{AtomicBool, Ordering}}, thread::JoinHandle};

use anyhow::Context;

use crate::*;

/// The address used when none is given: TCP on localhost, port 7878.
pub const DEFAULT_SOCKET_ADDRESS: &str = "127.0.0.1:7878";

/// Where a [`WorldSocket`] listens.  Parsed from `host:port` for TCP, or
/// `unix:<path>` for a unix socket on unix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketAddress {
    /// A TCP address, such as `127.0.0.1:7878`.
    Tcp(String),
    /// A unix socket's path.
    #[cfg(unix)]
    Unix(std::path::PathBuf)
}

impl FromStr for SocketAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        if let Some(path) = s.strip_prefix("unix:") {
            #[cfg(unix)]
            return Ok(SocketAddress::Unix(path.into()));
            #[cfg(not(unix))]
            anyhow::bail!("unix sockets aren't supported here: {path}");
        }
        Ok(SocketAddress::Tcp(s.to_string()))
    }
}

impl fmt::Display for SocketAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SocketAddress::Tcp(address) => f.write_str(address),
            #[cfg(unix)]
            SocketAddress::Unix(path) => write!(f, "unix:{}", path.display())
        }
    }
}

/// Serves a [`World`] to other programs, see the [module docs](crate::socket).  Each
/// connection gets its own thread.  Dropping it stops accepting connections, while
/// open connections are served until their clients disconnect.
///
/// ```no_run
/// # use anarchy::*;
/// let world = World::default();
/// let socket = WorldSocket::bind(&world, DEFAULT_SOCKET_ADDRESS)?;
/// // run the app, then `anarchy entities` lists its entities
/// # Ok::<_, anyhow::Error>(())
/// ```
pub struct WorldSocket {
    address: SocketAddress,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>
}

impl WorldSocket {
    /// Listens on `address`, see [`SocketAddress`], and serves `world` from a new thread.
    /// TCP port 0 picks a free port, see [`address`](Self::address).  A unix socket's
    /// file is replaced if it exists, and removed on drop.
    ///
    /// # Errors
    ///
    /// Errors if `address` doesn't parse or can't be listened on.
    pub fn bind(world: &World, address: &str) -> anyhow::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let (address, thread) = match address.parse()? {
            SocketAddress::Tcp(address) => {
                let listener = TcpListener::bind(&address).with_context(|| format!("listening on {address}"))?;
                let bound = SocketAddress::Tcp(listener.local_addr()?.to_string());
                (bound, accept(world, listener, &stop))
            },
            #[cfg(unix)]
            SocketAddress::Unix(path) => {
                let _ = std::fs::remove_file(&path);
                let listener = std::os::unix::net::UnixListener::bind(&path).with_context(|| format!("listening on {}", path.display()))?;
                (SocketAddress::Unix(path), accept(world, listener, &stop))
            }
        };
        Ok(Self { address, stop, thread: Some(thread) })
    }

    /// The address it listens on, with the actual port for TCP.
    pub fn address(&self) -> &SocketAddress { &self.address }
}

impl Drop for WorldSocket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // wake the accept loop with a connection of our own, so it sees the stop flag
        let woke = match &self.address {
            SocketAddress::Tcp(address) => TcpStream::connect(address).is_ok(),
            #[cfg(unix)]
            SocketAddress::Unix(path) => std::os::unix::net::UnixStream::connect(path).is_ok()
        };
        if woke && let Some(thread) = self.thread.take() { let _ = thread.join(); }
        #[cfg(unix)]
        if let SocketAddress::Unix(path) = &self.address { let _ = std::fs::remove_file(path); }
    }
}

/// A listening socket, TCP or unix.
trait Listener: Send + 'static {
    /// A connection to a client.
    type Stream: Read + Write + Send + 'static;
    /// Waits for the next connection.
    fn next(&self) -> std::io::Result<Self::Stream>;
}

impl Listener for TcpListener {
    type Stream = TcpStream;
    fn next(&self) -> std::io::Result<TcpStream> { self.accept().map(|(stream, _)| stream) }
}

#[cfg(unix)]
impl Listener for std::os::unix::net::UnixListener {
    type Stream = std::os::unix::net::UnixStream;
    fn next(&self) -> std::io::Result<Self::Stream> { self.accept().map(|(stream, _)| stream) }
}

/// Spawns the thread accepting connections on `listener` until `stop` is set.
fn accept(world: &World, listener: impl Listener, stop: &Arc<AtomicBool>) -> JoinHandle<()> {
    let world = world.clone();
    let stop = stop.clone();
    std::thread::spawn(move || loop {
        let stream = listener.next();
        if stop.load(Ordering::SeqCst) { break }
        let Ok(stream) = stream else { continue };
        let world = world.clone();
        std::thread::spawn(move || { let _ = serve(&world, stream); });
    })
}

/// Answers requests from `stream`, one JSON [`Request`] per line, until it closes.
/// [`WorldSocket`] calls this for each connection; call it yourself to serve a world
/// over another transport.
///
/// # Errors
///
/// Errors if reading from or writing to `stream` fails.  Malformed requests are
/// answered with [`Response::Error`].
pub fn serve(world: &World, stream: impl Read + Write) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 { return Ok(()) }
        if line.trim().is_empty() { continue }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => world.handle_request(request),
            Err(e) => Response::Error(format!("bad request: {e}"))
        };
        // writing through the reader is fine, it only buffers reads
        let stream = reader.get_mut();
        serde_json::to_writer(&mut *stream, &response)?;
        stream.write_all(b"\n")?;
        stream.flush()?;
    }
}
