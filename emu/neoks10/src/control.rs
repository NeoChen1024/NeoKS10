//! The control socket (docs/emulator.md, "Operator console", "The control
//! socket"): a Unix domain socket with mode 0600. One command for each
//! line, in the syntax of the console; one line of JSON for each reply.
//!
//! The server keeps the CTY output position of each connection for
//! `expect`, so that each client waits for text after its last match.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};

use reedline::Signal;
use serde_json::Value;

use crate::cmd::{self, Command, Units};
use crate::engine::{Reply, Request};
use crate::term;

/// The default socket path for clients: `NEOKS10_CONTROL`, else
/// `neoks10.sock` in the current directory.
pub fn client_path(arg: Option<PathBuf>) -> PathBuf {
    arg.or_else(|| std::env::var_os("NEOKS10_CONTROL").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("neoks10.sock"))
}

/// A path that `bind` and `connect` accept: a Unix socket path has at most
/// about 107 bytes, so use the path relative to the current directory if
/// it is shorter.
fn short(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let cwd = std::env::current_dir().unwrap_or_default();
    let rel = crate::engine::relative(&abs, &cwd);
    if rel.as_os_str().len() < abs.as_os_str().len() {
        rel
    } else {
        abs
    }
}

/// Listen on `path`. A socket file of a program that has ended is
/// removed; a socket that answers is an error.
pub fn listen(path: &Path, tx: Sender<Request>) -> Result<(), String> {
    let p = short(path);
    if let Ok(md) = std::fs::symlink_metadata(&p) {
        if !md.file_type().is_socket() {
            return Err(format!("{}: exists and is not a socket", path.display()));
        }
        if UnixStream::connect(&p).is_ok() {
            return Err(format!(
                "{}: another neoks10 uses this socket",
                path.display()
            ));
        }
        std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let l = UnixListener::bind(&p).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || serve(s, tx));
        }
    });
    Ok(())
}

/// Remove the socket file at the end of `run`.
pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(short(path));
}

fn serve(s: UnixStream, tx: Sender<Request>) {
    let Ok(mut w) = s.try_clone() else { return };
    let mut from = 0usize;
    for line in BufReader::new(s).lines() {
        let Ok(line) = line else { break };
        let reply = match cmd::parse(&line) {
            Ok(None) => continue,
            Ok(Some(Command::Cty)) => Reply::ok("", Value::Null),
            Ok(Some(cmd)) => {
                let (rtx, rrx) = channel();
                if tx
                    .send(Request::Command {
                        cmd,
                        from,
                        reply: rtx,
                    })
                    .is_err()
                {
                    Reply::err("the machine has stopped")
                } else {
                    match rrx.recv() {
                        Ok((r, f)) => {
                            from = f;
                            r
                        }
                        Err(_) => Reply::err("no reply"),
                    }
                }
            }
            Err(e) => Reply::err(e),
        };
        if writeln!(w, "{}", reply.json()).is_err() {
            break;
        }
    }
}

/// A connection of a client.
pub struct Client {
    w: UnixStream,
    r: BufReader<UnixStream>,
}

impl Client {
    pub fn connect(path: &Path) -> Result<Client, String> {
        let s = UnixStream::connect(short(path)).map_err(|e| format!("{}: {e}", path.display()))?;
        let r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
        Ok(Client { w: s, r })
    }

    /// Send one line; the reply as JSON.
    pub fn send(&mut self, line: &str) -> Result<Value, String> {
        writeln!(self.w, "{line}").map_err(|e| e.to_string())?;
        let mut l = String::new();
        self.r.read_line(&mut l).map_err(|e| e.to_string())?;
        if l.is_empty() {
            return Err("the connection closed".into());
        }
        serde_json::from_str(&l).map_err(|e| format!("bad reply: {e}"))
    }
}

fn reply_of(v: &Value) -> Reply {
    if v["ok"].as_bool() == Some(true) {
        Reply::ok(v["text"].as_str().unwrap_or(""), v["result"].clone())
    } else {
        Reply::err(v["error"].as_str().unwrap_or("error"))
    }
}

/// One word of a command line, quoted if it has spaces or quotes.
fn quote(a: &str) -> String {
    if !a.is_empty() && !a.chars().any(|c| c.is_whitespace() || c == '"') {
        return a.to_string();
    }
    format!("\"{}\"", a.replace('"', "\\\""))
}

/// The line with its file argument made absolute: the server has another
/// current directory than the client.
fn absolutize(line: &str) -> String {
    let path = match cmd::parse(line) {
        Ok(Some(Command::DiskAttach { path, .. }))
        | Ok(Some(Command::TapeMount { path, .. }))
        | Ok(Some(Command::ScriptRun(path))) => path,
        _ => return line.to_string(),
    };
    let Ok(mut w) = cmd::words(line) else {
        return line.to_string();
    };
    let abs = std::path::absolute(&path).unwrap_or_else(|_| PathBuf::from(&path));
    if let Some(x) = w.iter_mut().skip(2).find(|x| **x == path) {
        *x = abs.to_string_lossy().into_owned();
    }
    w.iter().map(|x| quote(x)).collect::<Vec<_>>().join(" ")
}

/// `neoks10 ctl [--control PATH] [--json] COMMAND...`.
pub fn ctl(args: &[String]) -> Result<i32, String> {
    let mut path = None;
    let mut raw = false;
    let mut words = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--control" if words.is_empty() => path = it.next().map(PathBuf::from),
            "--json" if words.is_empty() => raw = true,
            _ => words.push(quote(a)),
        }
    }
    if words.is_empty() {
        return Err("usage: neoks10 ctl [--control PATH] [--json] COMMAND...".into());
    }
    let mut c = Client::connect(&client_path(path))?;
    let v = c.send(&absolutize(&words.join(" ")))?;
    if raw {
        println!("{v}");
    } else {
        let r = reply_of(&v);
        let t = r.text.trim_end();
        if r.ok {
            if !t.is_empty() {
                println!("{t}");
            }
        } else {
            eprintln!("error: {t}");
        }
    }
    Ok(if v["ok"].as_bool() == Some(true) {
        0
    } else {
        1
    })
}

fn remote_units(c: &mut Client) -> Units {
    let mut u = Units::default();
    for k in 0..8 {
        u.disk.push((k, "-".into()));
        u.tape.push((k, "-".into()));
        u.line.push((k, "-".into()));
    }
    if let Ok(v) = c.send("show devices")
        && let Some(list) = v["result"]["units"].as_array()
    {
        for x in list {
            let (Some(kind), Some(n)) = (x["kind"].as_str(), x["unit"].as_u64()) else {
                continue;
            };
            let name = Path::new(x["image"].as_str().unwrap_or(""))
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let d = format!(
                "{name}{}",
                if x["read_only"].as_bool() == Some(true) {
                    " (ro)"
                } else {
                    ""
                }
            );
            let list = if kind == "disk" {
                &mut u.disk
            } else {
                &mut u.tape
            };
            if let Some(e) = list.get_mut(n as usize) {
                e.1 = d;
            }
        }
    }
    u
}

/// `neoks10 console [--control PATH]`: the prompt on another terminal.
/// Ctrl-D ends it; `quit` ends the emulator.
pub fn console(args: &[String]) -> Result<i32, String> {
    let path = match args {
        [] => None,
        [f, p] if f == "--control" => Some(PathBuf::from(p)),
        _ => return Err("usage: neoks10 console [--control PATH]".into()),
    };
    let path = client_path(path);
    let mut c = Client::connect(&path)?;
    let shared = std::sync::Arc::new(std::sync::Mutex::new(remote_units(&mut c)));
    let mut ed = term::editor(shared.clone());
    let prompt = term::prompt();
    println!("neoks10 console on {}; Ctrl-D ends it", path.display());
    loop {
        let line = match ed.read_line(&prompt) {
            Ok(Signal::Success(l)) => l,
            Ok(Signal::CtrlC(_)) => continue,
            Ok(_) => return Ok(0),
            Err(e) => return Err(e.to_string()),
        };
        match cmd::parse(&line) {
            Ok(None) | Ok(Some(Command::Cty)) => continue,
            Err(e) => {
                println!("{e}");
                continue;
            }
            Ok(Some(cmd)) => {
                let v = c.send(&absolutize(&line))?;
                let r = reply_of(&v);
                let t = r.text.trim_end();
                if r.ok {
                    if !t.is_empty() {
                        println!("{t}");
                    }
                } else {
                    println!("error: {t}");
                }
                if cmd == Command::Quit {
                    return Ok(0);
                }
                if let Ok(mut u) = shared.lock() {
                    *u = remote_units(&mut c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_files() {
        let cwd = std::env::current_dir().unwrap();
        let want = format!(
            "tape 0 mount {} --ro",
            quote(&cwd.join("x.tape").to_string_lossy())
        );
        assert_eq!(absolutize("tape 0 mount x.tape --ro"), want);
        assert_eq!(
            absolutize("tape 0 mount /a/x.tape"),
            "tape 0 mount /a/x.tape"
        );
        assert_eq!(absolutize("cpu show"), "cpu show");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("tape"), "tape");
        assert_eq!(quote("its\\r"), "its\\r");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(cmd::words(&quote("say \"hi\"")).unwrap(), ["say \"hi\""]);
    }
}
