//! [`SocketClient`], talking to a [`WorldSocket`] from another program.

use std::io::{BufRead, BufReader, Read, Write};

use anyhow::{Context, bail};

use crate::*;

/// A connection to a [`WorldSocket`], for tools and editors.
///
/// ```no_run
/// # use anarchy::*;
/// let mut client = SocketClient::connect(DEFAULT_SOCKET_ADDRESS)?;
/// let target = Target::Component { entity: 1, type_name: "Health".into() };
/// client.request(Request::Set { target, path: "current".into(), value: Value::Int(55) })?;
/// # Ok::<_, anyhow::Error>(())
/// ```
pub struct SocketClient {
    stream: BufReader<Box<dyn Stream>>
}

/// A connection's stream, TCP or unix.
trait Stream: Read + Write + Send {}
impl <S: Read + Write + Send> Stream for S {}

impl SocketClient {
    /// Connects to the socket at `address`, see [`SocketAddress`].
    ///
    /// # Errors
    ///
    /// Errors if `address` doesn't parse or nothing listens there.
    pub fn connect(address: &str) -> anyhow::Result<Self> {
        let stream: Box<dyn Stream> = match address.parse()? {
            SocketAddress::Tcp(address) => Box::new(std::net::TcpStream::connect(&address).with_context(|| format!("connecting to {address}"))?),
            #[cfg(unix)]
            SocketAddress::Unix(path) => Box::new(std::os::unix::net::UnixStream::connect(&path).with_context(|| format!("connecting to {}", path.display()))?)
        };
        Ok(Self { stream: BufReader::new(stream) })
    }

    /// Sends `request` and waits for the world's [`Response`], which may be a
    /// [`Response::Error`].
    ///
    /// # Errors
    ///
    /// Errors if the connection fails or the response doesn't parse.
    pub fn send(&mut self, request: &Request) -> anyhow::Result<Response> {
        let stream = self.stream.get_mut();
        serde_json::to_writer(&mut *stream, request)?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        let mut line = String::new();
        if self.stream.read_line(&mut line)? == 0 { bail!("the world closed the connection") }
        serde_json::from_str(&line).context("reading the response")
    }

    /// Sends `request` and returns the response, turning [`Response::Error`] into an error.
    ///
    /// # Errors
    ///
    /// Errors as [`send`](Self::send) does, or if the world answers with an error.
    pub fn request(&mut self, request: Request) -> anyhow::Result<Response> {
        match self.send(&request)? {
            Response::Error(e) => bail!("{e}"),
            response => Ok(response)
        }
    }
}
