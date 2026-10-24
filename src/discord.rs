//! Minimal Discord IPC client (no external RPC crate).
//!
//! Wire format: u32 LE opcode + u32 LE length + JSON payload.
//! Opcodes: 0 handshake, 1 frame, 2 close, 3 ping, 4 pong.

use crate::util;
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

const MAX_FRAME: usize = 4 * 1024 * 1024;

enum Stream {
    #[cfg(windows)]
    Pipe(std::fs::File),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}

impl Stream {
    fn open(index: u32) -> io::Result<Stream> {
        #[cfg(windows)]
        {
            let path = format!(r"\\.\pipe\discord-ipc-{index}");
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?;
            Ok(Stream::Pipe(file))
        }
        #[cfg(unix)]
        {
            let mut candidates = Vec::new();
            if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
                let dir = std::path::PathBuf::from(dir);
                candidates.push(dir.join(format!("discord-ipc-{index}")));
                candidates.push(
                    dir.join("snap.discord")
                        .join(format!("discord-ipc-{index}")),
                );
                candidates.push(
                    dir.join("app/com.discordapp.Discord")
                        .join(format!("discord-ipc-{index}")),
                );
            }
            if let Some(dir) = std::env::var_os("TMPDIR") {
                candidates.push(std::path::PathBuf::from(dir).join(format!("discord-ipc-{index}")));
            }
            candidates.push(std::path::PathBuf::from(format!("/tmp/discord-ipc-{index}")));

            let mut last = None;
            for path in candidates {
                match std::os::unix::net::UnixStream::connect(&path) {
                    Ok(sock) => {
                        let _ = sock.set_read_timeout(None);
                        return Ok(Stream::Unix(sock));
                    }
                    Err(e) => last = Some(e),
                }
            }
            Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no discord ipc")))
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = index;
            Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
        }
    }

    fn try_clone(&self) -> io::Result<Stream> {
        #[cfg(windows)]
        {
            match self {
                Stream::Pipe(f) => Ok(Stream::Pipe(f.try_clone()?)),
            }
        }
        #[cfg(unix)]
        {
            match self {
                Stream::Unix(s) => Ok(Stream::Unix(s.try_clone()?)),
            }
        }
        #[cfg(not(any(windows, unix)))]
        {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unsupported"))
        }
    }

    fn cancel(&self) {
        #[cfg(windows)]
        {
            let Stream::Pipe(f) = self;
            crate::sys::cancel_io(f);
        }
        #[cfg(unix)]
        {
            if let Stream::Unix(s) = self {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(windows)]
            Stream::Pipe(f) => f.read(buf),
            #[cfg(unix)]
            Stream::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(windows)]
            Stream::Pipe(f) => f.write(buf),
            #[cfg(unix)]
            Stream::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(windows)]
            Stream::Pipe(f) => f.flush(),
            #[cfg(unix)]
            Stream::Unix(s) => s.flush(),
        }
    }
}

struct Conn {
    writer: Stream,
    closed: Arc<AtomicBool>,
    reader: Option<thread::JoinHandle<()>>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        self.writer.cancel();
        // The reader thread unblocks via cancel()/shutdown().
        if let Some(handle) = self.reader.take() {
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}

pub struct Client {
    client_id: String,
    conn: Option<Conn>,
    counter: u64,
    retry_at: Instant,
    pub last_error: String,
    pub ever_connected: bool,
    warned_no_discord: bool,
}

impl Client {
    pub fn new(client_id: String) -> Client {
        Client {
            client_id,
            conn: None,
            counter: 0,
            retry_at: Instant::now(),
            last_error: String::new(),
            ever_connected: false,
            warned_no_discord: false,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.conn
            .as_ref()
            .map(|c| !c.closed.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Reconnect with backoff when Discord starts after us (or restarts).
    pub fn poll(&mut self) {
        let now = Instant::now();
        let lost = self
            .conn
            .as_ref()
            .map(|c| c.closed.load(Ordering::SeqCst))
            .unwrap_or(false);
        if lost {
            self.conn = None;
            self.retry_at = now + Duration::from_secs(3);
            crate::log_info!("Discord IPC connection closed");
        }
        if self.conn.is_some() || now < self.retry_at {
            return;
        }
        self.retry_at = now + Duration::from_secs(10);
        match self.try_connect() {
            Ok(()) => {
                self.ever_connected = true;
                self.warned_no_discord = false;
                self.last_error.clear();
                crate::log_info!("Connected to Discord");
            }
            Err(e) => {
                self.last_error = e.clone();
                if !self.warned_no_discord {
                    self.warned_no_discord = true;
                    crate::log_info!("Discord not reachable ({e}); retrying quietly");
                } else {
                    crate::log_debug!("Discord still unreachable: {e}");
                }
            }
        }
    }

    fn try_connect(&mut self) -> Result<(), String> {
        let mut last = String::from("no discord-ipc pipe found");
        for index in 0..10 {
            match Stream::open(index) {
                Ok(mut stream) => {
                    let handshake = json!({ "v": 1, "client_id": self.client_id });
                    let bytes = serde_json::to_vec(&handshake)
                        .map_err(|e| format!("handshake encode: {e}"))?;
                    write_frame(&mut stream, OP_HANDSHAKE, &bytes)
                        .map_err(|e| format!("handshake write: {e}"))?;

                    let reader_stream = stream
                        .try_clone()
                        .map_err(|e| format!("ipc clone: {e}"))?;
                    let closed = Arc::new(AtomicBool::new(false));
                    let closed_flag = closed.clone();
                    let reader = thread::Builder::new()
                        .name("discord-ipc-reader".to_string())
                        .spawn(move || read_loop(reader_stream, closed_flag))
                        .ok();

                    self.conn = Some(Conn {
                        writer: stream,
                        closed,
                        reader,
                    });
                    return Ok(());
                }
                Err(e) => last = e.to_string(),
            }
        }
        Err(last)
    }

    /// Returns true when the frame was handed to the OS.
    pub fn set_activity(&mut self, pid: u32, activity: Value) -> bool {
        let Some(conn) = self.conn.as_mut() else {
            return false;
        };
        if conn.closed.load(Ordering::SeqCst) {
            return false;
        }

        self.counter += 1;
        let nonce = format!("{}-{}", util::now_unix(), self.counter);
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": pid, "activity": activity },
            "nonce": nonce,
        });
        let bytes = match serde_json::to_vec(&payload) {
            Ok(b) => b,
            Err(e) => {
                crate::log_warn!("activity encode failed: {e}");
                return false;
            }
        };
        match write_frame(&mut conn.writer, OP_FRAME, &bytes) {
            Ok(()) => {
                crate::log_debug!("discord: activity frame written ({} bytes)", bytes.len());
                true
            }
            Err(e) => {
                crate::log_warn!("Discord write failed: {e}");
                self.last_error = e.to_string();
                self.conn = None;
                self.retry_at = Instant::now() + Duration::from_secs(5);
                false
            }
        }
    }

    pub fn clear(&mut self, pid: u32) {
        let _ = self.set_activity(pid, Value::Null);
    }
}

fn write_frame(stream: &mut Stream, opcode: u32, payload: &[u8]) -> io::Result<()> {
    let mut header = [0u8; 8];
    header[..4].copy_from_slice(&opcode.to_le_bytes());
    header[4..].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    stream.flush()
}

fn read_frame(stream: &mut Stream) -> io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header)?;
    let opcode = u32::from_le_bytes(header[..4].try_into().unwrap());
    let len = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    Ok((opcode, payload))
}

fn read_loop(mut stream: Stream, closed: Arc<AtomicBool>) {
    loop {
        match read_frame(&mut stream) {
            Ok((opcode, payload)) => match opcode {
                OP_PING => {
                    if write_frame(&mut stream, OP_PONG, &payload).is_err() {
                        break;
                    }
                }
                OP_CLOSE => {
                    crate::log_debug!("Discord sent CLOSE");
                    break;
                }
                OP_FRAME => {
                    if let Ok(value) = serde_json::from_slice::<Value>(&payload) {
                        if value.get("evt").and_then(|e| e.as_str()) == Some("ERROR") {
                            let detail = value
                                .get("data")
                                .map(|d| d.to_string())
                                .unwrap_or_default();
                            crate::log_warn!("Discord error: {detail}");
                        }
                    }
                }
                _ => {}
            },
            Err(_) => break,
        }
    }
    closed.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_header_is_little_endian() {
        let mut header = [0u8; 8];
        header[..4].copy_from_slice(&OP_FRAME.to_le_bytes());
        header[4..].copy_from_slice(&(5u32).to_le_bytes());
        assert_eq!(header, [1, 0, 0, 0, 5, 0, 0, 0]);
    }
}
