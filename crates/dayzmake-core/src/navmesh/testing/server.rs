//! A fake navmesh data server for tests: binds `127.0.0.1:0`, serves one client connection at a time
//! and answers message 2 with world params and message 3 from a list of tiles, with scripted faults.
//!
//! Self-contained (std and `dayzmake_core` only), so test crates of this workspace can also include it with
//! `#[path = ".../src/navmesh/testing/server.rs"] mod navmesh_server;`. It is public as
//! `dayzmake_core::navmesh::testing` with the feature `navmesh-testing`.
//!
//! ```ignore
//! let server = FakeServer::start(params, vec![tile], Faults::none())?;
//! let host = Host::parse(&server.addr().to_string())?;
//! // ... drive a client ...
//! assert_eq!(server.connections(), 1);
//! ```
//!
//! Dropping the server stops it: the listener is woken by a connection to itself and the client being
//! served is shut down, so no test waits on a timer.

#![allow(
    dead_code,
    reason = "shared by several test crates; not all of them use everything"
)]

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use dayzmake_core::navmesh::wire::{TileInput, WorldParams};

/// What the server does with one tile request (message 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fault {
    /// Answer with the requested tile.
    None,
    /// Close the connection without answering.
    Close,
    /// Answer with the neighbouring tile (x + 1) instead.
    WrongTile,
    /// Answer with a frame whose length field is 3.
    ShortFrame,
    /// Answer with a frame whose length field is 2^30 + 1, then close.
    HugeFrame,
    /// Answer with a tile message whose payload is malformed (truncated).
    Malformed,
    /// Answer with a valid frame header, then the payload one byte every 5 ms.
    Trickle,
    /// Answer with message id 4 (world params) instead of 5.
    WrongMessage,
}

/// The faults of the tile requests, in order of arrival (over all connections): `script[i]` for the
/// i-th request, then `then` for every later one.
#[derive(Debug, Clone)]
pub struct Faults {
    script: Vec<Fault>,
    then: Fault,
    slow: Option<((i32, i32), std::time::Duration)>,
    /// Connections `first..first + count` (counting from 0) are closed as soon as they are accepted.
    refuse: (usize, usize),
    /// A fault for one tile, whichever request it is.
    tile_fault: Option<((i32, i32), Fault)>,
}

impl Faults {
    /// No faults.
    #[must_use]
    pub fn none() -> Self {
        Self::always(Fault::None)
    }

    /// The same fault for every request.
    #[must_use]
    pub fn always(fault: Fault) -> Self {
        Self {
            script: Vec::new(),
            then: fault,
            slow: None,
            refuse: (0, 0),
            tile_fault: None,
        }
    }

    /// `script` for the first requests, then no fault.
    #[must_use]
    pub fn script(script: &[Fault]) -> Self {
        Self {
            script: script.to_vec(),
            then: Fault::None,
            slow: None,
            refuse: (0, 0),
            tile_fault: None,
        }
    }

    /// Every request for tile `(x, z)` gets `fault`; the other requests follow the other rules.
    #[must_use]
    pub fn fault_tile(mut self, tile: (i32, i32), fault: Fault) -> Self {
        self.tile_fault = Some((tile, fault));
        self
    }

    /// Closes the connections `first..first + count` (counting from 0) as soon as they are accepted.
    #[must_use]
    pub fn refuse_connections(mut self, first: usize, count: usize) -> Self {
        self.refuse = (first, count);
        self
    }

    /// Answers requests for tile `(x, z)` only after `delay` (a slow peer), whatever the other faults.
    #[must_use]
    pub fn slow_tile(mut self, tile: (i32, i32), delay: std::time::Duration) -> Self {
        self.slow = Some((tile, delay));
        self
    }

    fn at(&self, index: usize) -> Fault {
        self.script.get(index).copied().unwrap_or(self.then)
    }
}

struct Shared {
    params: WorldParams,
    tiles: Vec<TileInput>,
    faults: Faults,
    stop: AtomicBool,
    connections: AtomicUsize,
    requests: Mutex<Vec<(i32, i32)>>,
    current: Mutex<Option<TcpStream>>,
}

/// A running fake server. Dropping it stops it.
pub struct FakeServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl FakeServer {
    /// Starts a server for `params` and `tiles` with `faults`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the server cannot bind `127.0.0.1:0` or read its address.
    pub fn start(params: WorldParams, tiles: Vec<TileInput>, faults: Faults) -> io::Result<Self> {
        let context = |what: &'static str| {
            move |e: io::Error| io::Error::new(e.kind(), format!("fake server: {what}: {e}"))
        };
        let listener = TcpListener::bind("127.0.0.1:0").map_err(context("bind 127.0.0.1:0"))?;
        let addr = listener.local_addr().map_err(context("local address"))?;
        let shared = Arc::new(Shared {
            params,
            tiles,
            faults,
            stop: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            current: Mutex::new(None),
        });
        let thread = {
            let shared = Arc::clone(&shared);
            thread::spawn(move || accept_loop(&listener, &shared))
        };
        Ok(Self {
            addr,
            shared,
            thread: Some(thread),
        })
    }

    /// The address the server listens on.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Connections accepted so far.
    #[must_use]
    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }

    /// The `(x, z)` of every tile request received so far, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<(i32, i32)> {
        self.shared
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(s) = self
            .shared
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            // The client may be gone already.
            let _ = s.shutdown(Shutdown::Both);
        }
        // Wake the blocking accept; the connection is not counted because `stop` is set. A failed
        // connect means the listener is gone already, which is the goal.
        let _ = TcpStream::connect(self.addr);
        if let Some(t) = self.thread.take() {
            // A panic of the accept thread is not this test's failure; the tests assert on what the client saw.
            let _ = t.join();
        }
    }
}

fn accept_loop(listener: &TcpListener, shared: &Shared) {
    loop {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let number = shared.connections.fetch_add(1, Ordering::SeqCst);
        let (first, count) = shared.faults.refuse;
        if number >= first && number.saturating_sub(first) < count {
            // The peer may be gone: there is nothing to do if the shutdown fails.
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        if let Ok(clone) = stream.try_clone() {
            *shared
                .current
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(clone);
        }
        // `Drop` sets `stop` before it takes `current`, so one of the two sees the other's step.
        if shared.stop.load(Ordering::SeqCst) {
            // The peer may be gone: there is nothing to do if the shutdown fails.
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        // An I/O error just ends this client; the next one is accepted (the fake server has no
        // one to report it to).
        let _ = serve(&mut stream, shared);
        // The clone in `current` would keep the connection open, so close it explicitly (the peer may
        // be gone; nothing to do then).
        let _ = stream.shutdown(Shutdown::Both);
        shared
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

/// The largest request frame (length field) the server accepts. Requests are a few bytes; a larger
/// length ends the connection instead of allocating.
const MAX_REQUEST: u32 = 1 << 20;

fn read_frame(s: &mut TcpStream) -> io::Result<(u32, Vec<u8>)> {
    let mut head = [0u8; 8];
    s.read_exact(&mut head)?;
    let [l0, l1, l2, l3, i0, i1, i2, i3] = head;
    let len = u32::from_le_bytes([l0, l1, l2, l3]);
    let id = u32::from_le_bytes([i0, i1, i2, i3]);
    if !(4..=MAX_REQUEST).contains(&len) {
        return Err(io::Error::other(format!("invalid frame length {len}")));
    }
    let payload_len = usize::try_from(len - 4).map_err(io::Error::other)?;
    let mut payload = vec![0u8; payload_len];
    s.read_exact(&mut payload)?;
    Ok((id, payload))
}

fn write_frame(s: &mut TcpStream, id: u32, payload: &[u8]) -> io::Result<()> {
    let len = payload
        .len()
        .checked_add(4)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| io::Error::other("frame too large"))?;
    let mut out = Vec::new();
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(payload);
    s.write_all(&out)
}

fn serve(s: &mut TcpStream, shared: &Shared) -> io::Result<()> {
    s.set_nodelay(true)?;
    loop {
        let (id, payload) = read_frame(s)?;
        match id {
            1 => return Ok(()),
            2 => {
                let mut body = 10u32.to_le_bytes().to_vec();
                body.extend_from_slice(&shared.params.encode());
                write_frame(s, 4, &body)?;
            }
            3 => {
                let &[x0, x1, x2, x3, z0, z1, z2, z3, ..] = payload.as_slice() else {
                    return Err(io::Error::other("short tile request"));
                };
                let x = i32::from_le_bytes([x0, x1, x2, x3]);
                let z = i32::from_le_bytes([z0, z1, z2, z3]);
                let index = {
                    let mut requests = shared
                        .requests
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    requests.push((x, z));
                    requests.len().saturating_sub(1)
                };
                if let Some((tile, delay)) = shared.faults.slow
                    && tile == (x, z)
                {
                    // Pacing a slow fake peer, not waiting for state.
                    thread::sleep(delay);
                }
                if !answer(
                    s,
                    shared,
                    match shared.faults.tile_fault {
                        Some((tile, fault)) if tile == (x, z) => fault,
                        _ => shared.faults.at(index),
                    },
                    x,
                    z,
                )? {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

/// Answers one tile request; `false` ends the connection.
fn answer(s: &mut TcpStream, shared: &Shared, fault: Fault, x: i32, z: i32) -> io::Result<bool> {
    let find = |x: i32, z: i32| shared.tiles.iter().find(|t| t.x == x && t.z == z);
    match fault {
        Fault::Close => return Ok(false),
        Fault::ShortFrame => {
            s.write_all(&3u32.to_le_bytes())?;
            s.write_all(&[5, 0, 0])?;
        }
        Fault::HugeFrame => {
            s.write_all(&((1u32 << 30) + 1).to_le_bytes())?;
            s.write_all(&5u32.to_le_bytes())?;
            return Ok(false);
        }
        Fault::Malformed => {
            let mut bytes = find(x, z).map(TileInput::encode).unwrap_or_default();
            bytes.truncate(bytes.len().saturating_sub(5).max(10));
            write_frame(s, 5, &bytes)?;
        }
        Fault::Trickle => {
            let bytes = find(x, z).map(TileInput::encode).unwrap_or_default();
            let len = bytes
                .len()
                .checked_add(4)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| io::Error::other("frame too large"))?;
            s.write_all(&len.to_le_bytes())?;
            s.write_all(&5u32.to_le_bytes())?;
            for b in bytes {
                // Pacing a fake slow peer, not waiting for state.
                thread::sleep(std::time::Duration::from_millis(5));
                s.write_all(&[b])?;
            }
        }
        Fault::WrongMessage => write_frame(s, 4, &[10, 0, 0, 0])?,
        Fault::WrongTile => match x
            .checked_add(1)
            .and_then(|next| find(next, z))
            .or_else(|| find(x, z))
        {
            Some(t) => write_frame(s, 5, &t.encode())?,
            None => return Ok(false),
        },
        Fault::None => match find(x, z) {
            Some(t) => write_frame(s, 5, &t.encode())?,
            None => return Ok(false),
        },
    }
    Ok(true)
}
