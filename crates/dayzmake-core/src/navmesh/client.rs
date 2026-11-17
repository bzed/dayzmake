//! A TCP client for the navmesh data server (spec 3.1, 3.6).
//!
//! IPv4 only, `TCP_NODELAY`, one outstanding request per connection. A tile request that fails with a
//! network-class error is retried on a fresh connection ([`RetryPolicy`]); a malformed answer is not.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::navmesh::wire::{DEFAULT_PORT, TileInput, WorldParams, encode_tile_request};

const MSG_WORLD_PARAMS_REQUEST: u32 = 2;
const MSG_TILE_REQUEST: u32 = 3;
const MSG_WORLD_PARAMS: u32 = 4;
const MSG_TILE: u32 = 5;
/// Largest frame length field the server may send (spec 3.1).
pub(crate) const MAX_FRAME: u32 = 1 << 30;
/// Frames are read in chunks of at most this size, so a hostile length never allocates at once.
pub(crate) const READ_CHUNK: usize = 16 << 20;

/// How the client connects and retries a failed tile request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Attempts per tile (the first try included).
    pub attempts: u32,
    /// After failed attempt `n` (counting from 0) the client waits `delay_unit * (n + 1)`.
    pub delay_unit: Duration,
    /// Connect timeout.
    pub connect_timeout: Duration,
    /// How long each receive waits for a complete message.
    pub recv_timeout: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 5,
            delay_unit: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(10),
            recv_timeout: Duration::from_secs(600),
        }
    }
}

/// A data server address: a host name or IPv4 address and a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    name: String,
    port: u16,
}

impl Host {
    /// Parses `host` or `host:port`; the port defaults to [`DEFAULT_PORT`].
    ///
    /// # Errors
    ///
    /// Fails for an empty host or port, a port that is not a number in `1..=65535`, and more than
    /// one `:` (IPv6 is not supported).
    pub fn parse(text: &str) -> io::Result<Self> {
        let (name, port) = match text.split_once(':') {
            None => (text, DEFAULT_PORT),
            Some((name, port)) => {
                let port = port
                    .parse::<u16>()
                    .ok()
                    .filter(|p| *p != 0)
                    .ok_or_else(|| error!("invalid port in host \"{text}\""))?;
                (name, port)
            }
        };
        if name.is_empty() || name.contains(':') {
            return Err(error!("invalid host \"{text}\""));
        }
        Ok(Self {
            name: name.to_owned(),
            port,
        })
    }

    /// The host name or address.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.name, self.port)
    }
}

/// Why fetching a tile failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum FetchError {
    /// A network-class failure (connect, send, receive, timeout, peer closed, invalid frame, wrong
    /// message, wrong tile, unknown cell class) that persisted through every attempt.
    Network(io::Error),
    /// The tile payload is malformed. Not retried.
    Wire(io::Error),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(e) | Self::Wire(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for FetchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Network(e) | Self::Wire(e) => Some(e),
        }
    }
}

/// Reads `len` bytes in chunks of at most 16 MiB; the buffer only grows as data arrives.
pub(crate) fn read_chunked(r: &mut impl Read, len: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    while out.len() < len {
        let want = (len - out.len()).min(READ_CHUNK);
        let before = out.len();
        let got = r
            .take(u64::try_from(want).unwrap_or(u64::MAX))
            .read_to_end(&mut out)?;
        if got != want || out.len() != before + want {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "frame truncated",
            ));
        }
    }
    Ok(out)
}

/// Reads one frame: `u32 length`, `u32 messageId`, `length - 4` payload bytes.
///
/// # Errors
///
/// "invalid frame length N" for a length below 4 or above 2^30; an I/O error when the stream ends.
pub(crate) fn read_frame(r: &mut impl Read) -> io::Result<(u32, Vec<u8>)> {
    let mut head = [0u8; 8];
    r.read_exact(&mut head[..4])?;
    let length = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    if !(4..=MAX_FRAME).contains(&length) {
        return Err(error!("invalid frame length {length}"));
    }
    r.read_exact(&mut head[4..])?;
    let id = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    let payload_len =
        usize::try_from(length - 4).map_err(|_| error!("invalid frame length {length}"))?;
    Ok((id, read_chunked(r, payload_len)?))
}

/// Writes one frame.
pub(crate) fn write_frame(w: &mut impl Write, id: u32, payload: &[u8]) -> io::Result<()> {
    let length = payload
        .len()
        .checked_add(4)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= MAX_FRAME)
        .ok_or_else(|| error!("frame too large"))?;
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(payload);
    w.write_all(&out)?;
    w.flush()
}

/// A connection to a data server.
#[derive(Debug)]
pub struct Client {
    host: Host,
    policy: RetryPolicy,
    stream: Option<TcpStream>,
}

fn at_least_a_millisecond(d: Duration) -> Duration {
    d.max(Duration::from_millis(1))
}

fn open(host: &Host, policy: &RetryPolicy) -> io::Result<TcpStream> {
    let target = host.to_string();
    let hint = "start DayZDiag_x64.exe with -server -startNavmeshDataServer";
    let addr: SocketAddr = (host.name.as_str(), host.port)
        .to_socket_addrs()
        .map_err(|e| error!("cannot resolve {target}: {e}"))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| error!("no IPv4 address for {target}"))?;
    let stream = TcpStream::connect_timeout(&addr, at_least_a_millisecond(policy.connect_timeout))
        .map_err(|e| error!("cannot connect to {target}: {e}; {hint}"))?;
    stream.set_nodelay(true)?;
    let t = Some(at_least_a_millisecond(policy.recv_timeout));
    stream.set_read_timeout(t)?;
    stream.set_write_timeout(t)?;
    Ok(stream)
}

impl Client {
    /// Connects to `host`.
    ///
    /// # Errors
    ///
    /// Fails when the name does not resolve to an IPv4 address or the connection fails. The message
    /// names `host:port` and says to start `DayZDiag_x64.exe` with `-server -startNavmeshDataServer`.
    pub fn connect(host: &Host, policy: &RetryPolicy) -> io::Result<Self> {
        Ok(Self {
            host: host.clone(),
            policy: policy.clone(),
            stream: Some(open(host, policy)?),
        })
    }

    /// Requests the world params. One attempt, no retry.
    ///
    /// # Errors
    ///
    /// Fails on I/O errors, on an answer that is not message 4 ("expected world params, got message
    /// N") and on a malformed or wrong-version payload.
    pub fn world_params(&mut self) -> io::Result<WorldParams> {
        let result = self
            .exchange(MSG_WORLD_PARAMS_REQUEST, &[])
            .and_then(|(id, payload)| {
                if id == MSG_WORLD_PARAMS {
                    WorldParams::parse_message(&payload)
                } else {
                    Err(error!("expected world params, got message {id}"))
                }
            });
        if result.is_err() {
            self.stream = None;
        }
        result
    }

    /// Fetches tile `(x, z)`.
    ///
    /// A network-class failure closes the connection, waits `delay_unit * (attempt + 1)` and retries
    /// on a fresh connection; the last allowed attempt's failure is final ([`FetchError::Network`]).
    /// A malformed payload is a [`FetchError::Wire`] at once.
    ///
    /// # Errors
    ///
    /// See above. The answer must carry the requested `(x, z)` and a cell class that exists in
    /// `world`.
    pub fn tile(&mut self, x: i32, z: i32, world: &WorldParams) -> Result<TileInput, FetchError> {
        let attempts = self.policy.attempts.max(1);
        let mut attempt = 0u32;
        loop {
            match self.try_tile(x, z, world) {
                Ok(t) => return Ok(t),
                Err(e @ FetchError::Wire(_)) => return Err(e),
                Err(FetchError::Network(e)) => {
                    self.stream = None;
                    attempt += 1;
                    if attempt >= attempts {
                        return Err(FetchError::Network(e));
                    }
                    std::thread::sleep(self.policy.delay_unit.saturating_mul(attempt));
                }
            }
        }
    }

    fn try_tile(&mut self, x: i32, z: i32, world: &WorldParams) -> Result<TileInput, FetchError> {
        let request = encode_tile_request(x, z, &world.world);
        let (id, payload) = self
            .exchange(MSG_TILE_REQUEST, &request)
            .map_err(FetchError::Network)?;
        if id != MSG_TILE {
            return Err(FetchError::Network(error!(
                "expected tile, got message {id}"
            )));
        }
        let tile = TileInput::parse(&payload).map_err(FetchError::Wire)?;
        if tile.x != x || tile.z != z {
            return Err(FetchError::Network(error!(
                "asked for tile ({x},{z}), got ({},{})",
                tile.x, tile.z
            )));
        }
        let class = usize::try_from(tile.cell_class).unwrap_or(usize::MAX);
        if class >= world.cell_classes.len() {
            return Err(FetchError::Network(error!(
                "tile ({x},{z}): unknown cell class {}",
                tile.cell_class
            )));
        }
        Ok(tile)
    }

    /// Sends one request on the current connection (opening one if needed) and reads the answer.
    fn exchange(&mut self, id: u32, payload: &[u8]) -> io::Result<(u32, Vec<u8>)> {
        let stream = match &mut self.stream {
            Some(s) => s,
            slot => slot.insert(open(&self.host, &self.policy)?),
        };
        write_frame(stream, id, payload)?;
        read_frame(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(length: u32, id: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = length.to_le_bytes().to_vec();
        v.extend_from_slice(&id.to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn read_frame_rules() {
        let (id, p) = read_frame(&mut Cursor::new(frame(7, 5, &[1, 2, 3]))).expect("ok");
        assert_eq!((id, p), (5, vec![1, 2, 3]));
        let (id, p) = read_frame(&mut Cursor::new(frame(4, 9, &[]))).expect("empty");
        assert_eq!((id, p), (9, vec![]));
        for bad in [0u32, 3, MAX_FRAME + 1, u32::MAX] {
            let e = read_frame(&mut Cursor::new(frame(bad, 5, &[0; 8]))).expect_err("bad");
            assert!(e.to_string().contains("invalid frame"), "{e}");
        }
    }

    #[test]
    fn read_frame_longer_than_the_stream_is_truncated_not_allocated() {
        // Announces 1 GiB, delivers 5 bytes: an error, and only 5 bytes were ever buffered.
        let e = read_frame(&mut Cursor::new(frame(MAX_FRAME, 5, &[1; 5]))).expect_err("short");
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn read_chunked_crosses_chunk_boundaries() {
        let data = vec![7u8; READ_CHUNK + 5];
        let out = read_chunked(&mut Cursor::new(&data), data.len()).expect("ok");
        assert_eq!(out.len(), data.len());
        assert!(read_chunked(&mut Cursor::new(&data), data.len() + 1).is_err());
    }

    #[test]
    fn write_frame_layout() {
        let mut out = Vec::new();
        write_frame(&mut out, 3, &[9, 8]).expect("ok");
        assert_eq!(out, frame(6, 3, &[9, 8]));
    }
}
