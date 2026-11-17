//! A fake navmesh data server for tests: binds `127.0.0.1:0`, serves one client connection at a time
//! and answers message 2 with world params and message 3 from a list of tiles, with scripted faults.
//!
//! Self-contained (std and `dayzmake_core` only), so other test crates can include it with
//! `#[path = ".../support/navmesh_server.rs"] mod navmesh_server;`.
//!
//! ```ignore
//! let server = FakeServer::start(params, vec![tile], Faults::none());
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
    /// Answer with message id 4 (world params) instead of 5.
    WrongMessage,
}

/// The faults of the tile requests, in order of arrival (over all connections): `script[i]` for the
/// i-th request, then `then` for every later one.
#[derive(Debug, Clone)]
pub struct Faults {
    script: Vec<Fault>,
    then: Fault,
}

impl Faults {
    /// No faults.
    pub fn none() -> Self {
        Self::always(Fault::None)
    }

    /// The same fault for every request.
    pub fn always(fault: Fault) -> Self {
        Self {
            script: Vec::new(),
            then: fault,
        }
    }

    /// `script` for the first requests, then no fault.
    pub fn script(script: &[Fault]) -> Self {
        Self {
            script: script.to_vec(),
            then: Fault::None,
        }
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
    pub fn start(params: WorldParams, tiles: Vec<TileInput>, faults: Faults) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
        let addr = listener.local_addr().expect("local addr");
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
        Self {
            addr,
            shared,
            thread: Some(thread),
        }
    }

    /// The address the server listens on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Connections accepted so far.
    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }

    /// The `(x, z)` of every tile request received so far, in order.
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
        // Wake the blocking accept; the connection is not counted because `stop` is set.
        let _ = TcpStream::connect(self.addr);
        if let Some(t) = self.thread.take() {
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
        shared.connections.fetch_add(1, Ordering::SeqCst);
        if let Ok(clone) = stream.try_clone() {
            *shared
                .current
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(clone);
        }
        // `Drop` sets `stop` before it takes `current`, so one of the two sees the other's step.
        if shared.stop.load(Ordering::SeqCst) {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        // An I/O error just ends this client; the next one is accepted.
        let _ = serve(&mut stream, shared);
        // The clone in `current` would keep the connection open, so close it explicitly.
        let _ = stream.shutdown(Shutdown::Both);
        shared
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

fn read_frame(s: &mut TcpStream) -> io::Result<(u32, Vec<u8>)> {
    let mut head = [0u8; 8];
    s.read_exact(&mut head)?;
    let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let id = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    let mut payload = vec![0u8; usize::try_from(len.saturating_sub(4)).expect("small")];
    s.read_exact(&mut payload)?;
    Ok((id, payload))
}

fn write_frame(s: &mut TcpStream, id: u32, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len() + 4).expect("small");
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
                let x = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let z = i32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let index = {
                    let mut requests = shared
                        .requests
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    requests.push((x, z));
                    requests.len() - 1
                };
                if !answer(s, shared, shared.faults.at(index), x, z)? {
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
        Fault::WrongMessage => write_frame(s, 4, &[10, 0, 0, 0])?,
        Fault::WrongTile => match find(x + 1, z).or_else(|| find(x, z)) {
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
