//! The host side of the DZ11 lines (docs/emulator.md, "DZ11 lines"): one
//! TCP listener for each line, one connection on a line at a time, and
//! the Telnet protocol (RFC 854) in character mode.
//!
//! The threads here only move bytes. The machine thread decides about
//! each connection (`Request::DzConnect`), so that a connection, its
//! input and its end are inputs of the host layer with an instruction
//! count, and `--record` writes them.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use crate::engine::Request;

/// Telnet commands (RFC 854) and options.
const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
/// RFC 857.
const ECHO: u8 = 1;
/// RFC 858.
const SGA: u8 = 3;

/// At the start of a connection: the server echoes (WILL ECHO), and both
/// sides suppress go-ahead (character mode).
pub const GREETING: [u8; 9] = [IAC, WILL, ECHO, IAC, WILL, SGA, IAC, DO, SGA];

/// The receive side of the Telnet protocol: the data bytes of a stream,
/// without the commands. CR LF and CR NUL are CR, the end of a line
/// on a terminal (RFC 854, "The NVT printer and keyboard").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Telnet {
    #[default]
    Data,
    /// After IAC.
    Iac,
    /// After IAC WILL, WONT, DO or DONT: the option byte follows.
    Option,
    /// In a subnegotiation (IAC SB ... IAC SE).
    Sub,
    SubIac,
    /// After a CR.
    Cr,
}

impl Telnet {
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) {
        for &b in input {
            *self = match (*self, b) {
                (Telnet::Data | Telnet::Cr, IAC) => Telnet::Iac,
                (Telnet::Cr, 0 | b'\n') => Telnet::Data,
                (Telnet::Data | Telnet::Cr, b'\r') => {
                    out.push(b'\r');
                    Telnet::Cr
                }
                (Telnet::Data | Telnet::Cr, b) => {
                    out.push(b);
                    Telnet::Data
                }
                (Telnet::Iac, IAC) => {
                    out.push(IAC);
                    Telnet::Data
                }
                (Telnet::Iac, WILL | WONT | DO | DONT) => Telnet::Option,
                (Telnet::Iac, SB) => Telnet::Sub,
                (Telnet::Iac, _) | (Telnet::Option, _) => Telnet::Data,
                (Telnet::Sub, IAC) => Telnet::SubIac,
                (Telnet::Sub, _) => Telnet::Sub,
                (Telnet::SubIac, SE) => Telnet::Data,
                (Telnet::SubIac, _) => Telnet::Sub,
            }
        }
    }
}

/// The send side: a data byte 255 is IAC IAC.
pub fn escape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for &b in data {
        out.push(b);
        if b == IAC {
            out.push(IAC);
        }
    }
    out
}

/// A listener of one line. Dropping it stops the listener.
pub struct Listener {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Listen on `addr` for line `line`.
pub fn listen(line: usize, addr: &str, tx: Sender<Request>) -> Result<Listener, String> {
    let l = TcpListener::bind(addr).map_err(|e| format!("dz line {line}: {addr}: {e}"))?;
    let local = l.local_addr().map_err(|e| e.to_string())?;
    l.set_nonblocking(true).map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match l.accept() {
                Ok((s, peer)) => {
                    let tx = tx.clone();
                    std::thread::spawn(move || connection(line, s, peer, tx));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    });
    Ok(Listener { addr: local, stop })
}

fn connection(line: usize, mut s: TcpStream, peer: SocketAddr, tx: Sender<Request>) {
    let _ = s.set_nonblocking(false);
    let _ = s.set_nodelay(true);
    let (otx, orx) = channel::<Vec<u8>>();
    let (rtx, rrx) = channel();
    if tx
        .send(Request::DzConnect {
            line,
            peer,
            out: otx,
            reply: rtx,
        })
        .is_err()
    {
        return;
    }
    let id = match rrx.recv() {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => {
            let _ = s.write_all(format!("\r\nneoks10: {e}\r\n").as_bytes());
            return;
        }
        Err(_) => return,
    };
    let _ = s.write_all(&GREETING);
    let _ = s.write_all(format!("\r\nneoks10: DZ11 line {line}\r\n").as_bytes());
    // The writer: the output of the line, until the machine thread ends
    // the connection (it drops the sender).
    let Ok(mut w) = s.try_clone() else { return };
    std::thread::spawn(move || {
        for b in orx {
            if w.write_all(&escape(&b)).is_err() {
                break;
            }
        }
        let _ = w.shutdown(Shutdown::Both);
    });
    let mut t = Telnet::default();
    let mut buf = [0u8; 1024];
    loop {
        let n = match s.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut data = Vec::new();
        t.feed(&buf[..n], &mut data);
        if !data.is_empty() && tx.send(Request::DzInput { line, id, data }).is_err() {
            break;
        }
    }
    let _ = tx.send(Request::DzHangup { line, id });
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 854: IAC IAC is a data byte 255; IAC DO, WILL, WONT, DONT take
    // one option byte; IAC SB ... IAC SE is a subnegotiation; IAC with
    // another command (here NOP, 241) takes no byte. CR NUL and CR LF are
    // CR.
    #[test]
    fn telnet_input() {
        let mut t = Telnet::default();
        let mut out = Vec::new();
        t.feed(
            &[
                b'a', IAC, IAC, IAC, DO, ECHO, b'b', IAC, SB, 24, 0, b'x', IAC, SE,
            ],
            &mut out,
        );
        t.feed(
            &[IAC, 241, b'c', b'\r', 0, b'd', b'\r', b'\n', b'\r'],
            &mut out,
        );
        assert_eq!(out, [b'a', IAC, b'b', b'c', b'\r', b'd', b'\r', b'\r']);
        // A command split between two reads.
        let mut t = Telnet::default();
        let mut out = Vec::new();
        t.feed(&[IAC], &mut out);
        t.feed(&[WILL], &mut out);
        t.feed(&[SGA, b'e'], &mut out);
        assert_eq!(out, b"e");
    }

    #[test]
    fn telnet_output() {
        assert_eq!(escape(&[1, IAC, 2]), [1, IAC, IAC, 2]);
    }
}
