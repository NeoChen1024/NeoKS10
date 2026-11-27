//! The machine thread (docs/emulator.md, "Threads and determinism").
//!
//! One thread owns the `Machine`. The terminal, the control socket and
//! the scripts send requests through a channel; the thread does them
//! between two instructions. Each request that changes the guest is an
//! `Input`: it gets the instruction count at which the guest sees it, and
//! `--record` writes it. A replay feeds the recorded inputs at the same
//! counts, so it repeats the run without the terminal or the socket.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use ks10::{Machine, Stop, boot};
use pdp10::{Half, W36};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cmd::{Command, Space, Units};
use crate::script::Script;

/// The instructions between two looks at the requests.
const BATCH: u64 = 10_000;

/// The default limit of `expect`, in instructions.
pub const EXPECT_STEPS: u64 = 1_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Disk,
    Tape,
}

/// An input that changes the guest, as the record file has it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    Cty(Vec<u8>),
    Attach {
        kind: Kind,
        unit: usize,
        path: PathBuf,
        read_only: bool,
        /// The SHA-256 of the image when the record was made: a replay
        /// checks that its image is the same.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    Detach {
        kind: Kind,
        unit: usize,
    },
    Boot {
        kind: Kind,
        unit: usize,
    },
    Halt,
    Continue,
    Step(u64),
    Start {
        space: Space,
        addr: u32,
    },
    Deposit {
        space: Space,
        addr: u32,
        word: u64,
    },
    Mark(String),
    /// The end of the run (`quit`); a replay ends here.
    Quit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Stamped {
    count: u64,
    input: Input,
}

/// The reply to a command.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub ok: bool,
    /// For a person.
    pub text: String,
    /// For a program.
    pub result: Value,
}

impl Reply {
    pub fn ok(text: impl Into<String>, result: Value) -> Reply {
        Reply {
            ok: true,
            text: text.into(),
            result,
        }
    }

    pub fn err(text: impl Into<String>) -> Reply {
        Reply {
            ok: false,
            text: text.into(),
            result: Value::Null,
        }
    }

    /// The line of the control protocol.
    pub fn json(&self) -> String {
        let v = if self.ok {
            json!({"ok": true, "result": self.result, "text": self.text})
        } else {
            json!({"ok": false, "error": self.text})
        };
        v.to_string()
    }
}

/// What the machine thread sends to the terminal and the clients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// CTY output, as the 8080 printed it.
    Cty(Vec<u8>),
    /// A message of the emulator (a halt, the end of a script).
    Notice(String),
    /// Show what waits (the terminal sends this to its own printer when
    /// the prompt closes).
    Flush,
}

pub enum Request {
    /// A command; `from` is the CTY output position of the client for
    /// `expect`. The reply has the new position for `expect`.
    Command {
        cmd: Command,
        from: usize,
        reply: Sender<(Reply, usize)>,
    },
    /// Keys from the terminal.
    Keys(Vec<u8>),
    /// Send the CTY output and the notices to this channel from now on.
    Subscribe(Sender<Out>),
    /// The units and their images, for the completion.
    Units(Sender<Units>),
}

/// A running script.
struct ScriptRun {
    s: Script,
    /// The CTY output position after the last match.
    from: usize,
    /// The instruction count of the last step.
    at: u64,
    /// An expect that waits: text, limit, line.
    wait: Option<(Vec<u8>, u64, usize)>,
    /// The script of `run --script`: its failure ends the run. A script
    /// of `script run` only stops, so that the other users go on.
    startup: bool,
}

struct Expect {
    text: Vec<u8>,
    from: usize,
    until: u64,
    reply: Sender<(Reply, usize)>,
}

/// How the run follows the wall clock (docs/emulator.md, "Time").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// No wait.
    Fast,
    /// Wait for the time of the idle passes only: full speed while the
    /// guest works, a sleep of the host while it is idle.
    Idle,
    /// Wait so that the virtual time does not run ahead of the wall
    /// clock.
    Realtime,
}

pub struct Options {
    pub ns_per_instruction: u64,
    pub pace: Pace,
    /// Do the passes of the idle loop at once (`Machine::step_idle`).
    pub idle: bool,
    pub record: Option<PathBuf>,
    pub replay: Option<PathBuf>,
}

pub struct Engine {
    m: Machine,
    running: bool,
    /// Why the processor stopped last.
    stopped: Option<String>,
    opts: Options,
    record: Option<BufWriter<File>>,
    replay: VecDeque<Stamped>,
    subscribers: Vec<Sender<Out>>,
    /// The CTY output that the subscribers have.
    sent: usize,
    expects: Vec<Expect>,
    images: [[Option<(PathBuf, bool)>; 8]; 2],
    /// The wall clock and instruction count of the last start, for
    /// `--pace realtime`.
    base: (Instant, u64),
    script: Option<ScriptRun>,
    /// The passes of the idle loop done at once.
    skipped: u64,
    pub quit: bool,
    /// The exit status of `run`: 1 after a script error.
    pub exit: i32,
}

fn kind_index(k: Kind) -> usize {
    match k {
        Kind::Disk => 0,
        Kind::Tape => 1,
    }
}

impl Engine {
    pub fn new(opts: Options) -> Result<Engine, String> {
        let record = match &opts.record {
            Some(p) => Some(BufWriter::new(
                File::create(p).map_err(|e| format!("{}: {e}", p.display()))?,
            )),
            None => None,
        };
        let mut replay = VecDeque::new();
        if let Some(p) = &opts.replay {
            let f = File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
            for (n, line) in BufReader::new(f).lines().enumerate() {
                let line = line.map_err(|e| format!("{}: {e}", p.display()))?;
                let mut s: Stamped = serde_json::from_str(&line)
                    .map_err(|e| format!("{}:{}: {e}", p.display(), n + 1))?;
                if let Input::Attach { path, .. } = &mut s.input
                    && path.is_relative()
                {
                    *path = p.parent().unwrap_or(Path::new("")).join(&*path);
                }
                replay.push_back(s);
            }
        }
        Ok(Engine {
            m: Machine::new(),
            running: false,
            stopped: None,
            opts,
            record,
            replay,
            subscribers: Vec::new(),
            sent: 0,
            expects: Vec::new(),
            images: Default::default(),
            base: (Instant::now(), 0),
            script: None,
            skipped: 0,
            quit: false,
            exit: 0,
        })
    }

    pub fn replaying(&self) -> bool {
        self.opts.replay.is_some()
    }

    /// The units for the completion.
    pub fn units(&self) -> Units {
        let list = |k: usize| {
            (0..8)
                .map(|u| {
                    let d = match &self.images[k][u] {
                        Some((p, ro)) => {
                            let name = p.file_name().unwrap_or_default().to_string_lossy();
                            format!("{name}{}", if *ro { " (ro)" } else { "" })
                        }
                        None => "-".into(),
                    };
                    (u, d)
                })
                .collect()
        };
        Units {
            disk: list(0),
            tape: list(1),
        }
    }

    /// Do an input now: record it, then apply it. The record has image
    /// paths relative to its own directory, so that it has no directory
    /// of the host and works at another place.
    pub fn input(&mut self, i: Input) -> Result<String, String> {
        if let Some(w) = &mut self.record {
            let mut input = i.clone();
            if let Input::Attach { path, sha256, .. } = &mut input {
                *sha256 = Some(sha256_file(path)?);
                let base = self.opts.record.as_deref().and_then(Path::parent);
                let base = base
                    .filter(|b| !b.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                let base = std::path::absolute(base).unwrap_or_default();
                *path = relative(path, &base);
            }
            let s = Stamped {
                count: self.m.count,
                input,
            };
            let line = serde_json::to_string(&s).map_err(|e| e.to_string())?;
            writeln!(w, "{line}")
                .and_then(|()| w.flush())
                .map_err(|e| format!("record: {e}"))?;
        }
        self.apply(i)
    }

    fn open(path: &Path, read_only: bool) -> Result<File, String> {
        OpenOptions::new()
            .read(true)
            .write(!read_only)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    fn apply(&mut self, i: Input) -> Result<String, String> {
        match i {
            Input::Cty(b) => {
                self.m.console.input.extend(b);
                Ok(String::new())
            }
            Input::Attach {
                kind,
                unit,
                path,
                read_only,
                ..
            } => {
                let f = Self::open(&path, read_only)?;
                match kind {
                    Kind::Disk => self.m.io.disk.attach(unit, f, read_only),
                    Kind::Tape => self.m.io.tape.mount(unit, f, read_only),
                }
                let t = format!("{} {unit}: {}", kind_name(kind), path.display());
                self.images[kind_index(kind)][unit] = Some((path, read_only));
                Ok(t)
            }
            Input::Detach { kind, unit } => {
                match kind {
                    Kind::Disk => self.m.io.disk.drives[unit] = None,
                    Kind::Tape => self.m.io.tape.unmount(unit),
                }
                self.images[kind_index(kind)][unit] = None;
                Ok(format!("{} {unit}: no image", kind_name(kind)))
            }
            Input::Boot { kind, unit } => {
                let r = match kind {
                    Kind::Disk => boot::boot_disk(&mut self.m, unit as u16),
                    Kind::Tape => boot::boot_tape(&mut self.m, unit as u16),
                };
                r.map_err(|e| format!("boot: {e:?}"))?;
                self.start_running();
                Ok(format!("booted from {} {unit}", kind_name(kind)))
            }
            Input::Halt => {
                self.running = false;
                self.stopped = Some("halted by the operator".into());
                self.fail_expects("expect: the processor was halted");
                Ok(format!("halted at PC {}", self.m.pc))
            }
            Input::Continue => {
                self.start_running();
                Ok(format!("running from PC {}", self.m.pc))
            }
            Input::Step(n) => {
                for _ in 0..n {
                    if let Err(s) = self.m.step() {
                        self.note_stop(s);
                        break;
                    }
                }
                Ok(format!(
                    "PC {} after {} instructions",
                    self.m.pc, self.m.count
                ))
            }
            Input::Start { space, addr } => {
                if space != Space::Phys && self.m.paging {
                    return Err("cpu start: give a physical address, or one without paging".into());
                }
                self.m.pc = Half::wrap(u64::from(addr));
                self.start_running();
                Ok(format!("running from PC {}", self.m.pc))
            }
            Input::Deposit { space, addr, word } => {
                let p = self
                    .physical(space, addr)
                    .ok_or("no access to this address")?;
                self.m
                    .phys_write(p, W36::wrap(word))
                    .map_err(|_| format!("{p:o}: no memory"))?;
                Ok(format!("{p:o}/ {}", W36::wrap(word).halves_text()))
            }
            Input::Mark(t) => Ok(format!("mark at {}: {t}", self.m.count)),
            Input::Quit => {
                self.quit = true;
                Ok("quit".into())
            }
        }
    }

    fn start_running(&mut self) {
        self.running = true;
        self.stopped = None;
        self.base = (Instant::now(), self.m.count);
    }

    fn physical(&self, space: Space, a: u32) -> Option<u32> {
        match space {
            Space::Phys => Some(a),
            Space::Exec | Space::User => {
                ks10::pager::peek(&self.m, Half::wrap(u64::from(a)), space == Space::User)
            }
        }
    }

    fn note_stop(&mut self, s: Stop) {
        self.running = false;
        let t = match s {
            Stop::Halt { pc, code } => format!("halt code {code:o} at PC {pc}"),
            Stop::Unimplemented { pc, word } => {
                format!(
                    "unimplemented instruction {} at PC {pc}",
                    word.halves_text()
                )
            }
            Stop::Limit => "step limit".into(),
        };
        self.notice(format!("neoks10: processor stopped: {t}"));
        self.stopped = Some(t);
        self.fail_expects("expect: the processor stopped");
    }

    /// The expects of the clients cannot succeed while the processor
    /// stops: the instruction count does not move.
    fn fail_expects(&mut self, why: &str) {
        for e in std::mem::take(&mut self.expects) {
            let _ = e.reply.send((Reply::err(why), e.from));
        }
    }

    fn notice(&mut self, t: String) {
        self.subscribers
            .retain(|s| s.send(Out::Notice(t.clone())).is_ok());
    }

    /// A command of the console. The reply goes to `reply`, at once or,
    /// for `expect`, when the text appears.
    pub fn command(&mut self, cmd: Command, from: usize, reply: Sender<(Reply, usize)>) {
        if let Some(r) = self.command_now(cmd, from, &reply) {
            let _ = reply.send(r);
        }
    }

    fn command_now(
        &mut self,
        cmd: Command,
        from: usize,
        tx: &Sender<(Reply, usize)>,
    ) -> Option<(Reply, usize)> {
        if cmd.needs_halt() && self.running {
            return Some((
                Reply::err("the processor runs: give \"cpu halt\" first"),
                from,
            ));
        }
        if self.replaying() && changes_guest(&cmd) {
            return Some((Reply::err("a replay takes no inputs"), from));
        }
        let r = |x: Result<String, String>| match x {
            Ok(t) => Reply::ok(t, Value::Null),
            Err(e) => Reply::err(e),
        };
        let reply = match cmd {
            Command::Help => Reply::ok(crate::cmd::help(), Value::Null),
            Command::Cty => Reply::ok("", Value::Null),
            Command::Quit => {
                // A replay ends at its recorded quit, not at a command.
                self.quit = true;
                if self.replaying() {
                    Reply::ok("quit", Value::Null)
                } else {
                    r(self.input(Input::Quit))
                }
            }
            Command::CpuShow => self.cpu_show(),
            Command::CpuHalt => r(self.input(Input::Halt)),
            Command::CpuContinue => r(self.input(Input::Continue)),
            Command::CpuStep(n) => r(self.input(Input::Step(n))),
            Command::CpuStart(space, addr) => r(self.input(Input::Start { space, addr })),
            Command::BootDisk(u) => r(self.input(Input::Boot {
                kind: Kind::Disk,
                unit: u,
            })),
            Command::BootTape(u) => r(self.input(Input::Boot {
                kind: Kind::Tape,
                unit: u,
            })),
            Command::DiskAttach {
                unit,
                path,
                read_only,
            } => r(self.input(Input::Attach {
                kind: Kind::Disk,
                unit,
                path: absolute(&path),
                read_only,
                sha256: None,
            })),
            Command::DiskDetach(u) => r(self.input(Input::Detach {
                kind: Kind::Disk,
                unit: u,
            })),
            Command::TapeMount {
                unit,
                path,
                read_only,
            } => r(self.input(Input::Attach {
                kind: Kind::Tape,
                unit,
                path: absolute(&path),
                read_only,
                sha256: None,
            })),
            Command::TapeUnmount(u) => r(self.input(Input::Detach {
                kind: Kind::Tape,
                unit: u,
            })),
            Command::MemExamine(space, a, b) => self.examine(space, a, b),
            Command::MemDeposit(space, addr, word) => {
                r(self.input(Input::Deposit { space, addr, word }))
            }
            Command::CtySend(b) => match self.input(Input::Cty(b.clone())) {
                Ok(_) => Reply::ok("", json!({"bytes": b.len()})),
                Err(e) => Reply::err(e),
            },
            Command::CtyRead(f) => {
                let out = &self.m.console.output;
                let start = f.map_or(0, |f| f as usize).min(out.len());
                let text: String = out[start..]
                    .iter()
                    .map(|&c| char::from(c & 0o177))
                    .collect();
                Reply::ok(text.clone(), json!({"text": text, "next": out.len()}))
            }
            Command::Expect { text, steps } => {
                let from = from.min(self.m.console.output.len());
                if let Some(end) = find(&self.m.console.output[from..], &text) {
                    let to = from + end;
                    return Some((Reply::ok("", json!({"next": to})), to));
                }
                if !self.running {
                    return Some((Reply::err("expect: the processor does not run"), from));
                }
                // The reply comes when the text appears (`check_expects`).
                self.expects.push(Expect {
                    text,
                    from,
                    until: self.m.count.saturating_add(steps.unwrap_or(EXPECT_STEPS)),
                    reply: tx.clone(),
                });
                return None;
            }
            Command::ShowDevices => self.show_devices(),
            Command::RecordMark(t) => r(self.input(Input::Mark(t))),
            Command::ScriptRun(p) => r(self.load_script(&p)),
            Command::ScriptStop => match self.script.take() {
                Some(s) => Reply::ok(format!("{}: stopped", s.s.name), Value::Null),
                None => Reply::err("no script runs"),
            },
            Command::ScriptShow => match &self.script {
                Some(s) => {
                    let next = s.s.steps.front().map(|x| x.0);
                    let state = match (&s.wait, next) {
                        (Some((t, _, l)), _) => format!(
                            "line {l}: waits for \"{}\"",
                            String::from_utf8_lossy(t).escape_debug()
                        ),
                        (None, Some(l)) => format!("next: line {l}"),
                        (None, None) => "at the end".into(),
                    };
                    Reply::ok(
                        format!("{}: {state}", s.s.name),
                        json!({"script": s.s.name, "steps": s.s.steps.len()}),
                    )
                }
                None => Reply::ok("no script runs", Value::Null),
            },
        };
        Some((reply, from))
    }
}

impl Engine {
    fn cpu_show(&self) -> Reply {
        let m = &self.m;
        let state = if self.running {
            "running".to_string()
        } else {
            format!(
                "stopped ({})",
                self.stopped.as_deref().unwrap_or("not started")
            )
        };
        let acs: Vec<String> = m.ac.iter().map(|w| w.halves_text()).collect();
        let ac_text: String = acs
            .chunks(4)
            .enumerate()
            .map(|(k, c)| format!("\nAC {:02o}: {}", 4 * k, c.join("  ")))
            .collect();
        let text = format!(
            "PC {}  flags {:06o}  {state}\ninstructions {} ({} idle passes at once)  paging {}  PI {}\nEBR {}  UBR {}{ac_text}",
            m.pc,
            m.flags.0,
            m.count,
            self.skipped,
            if m.paging { "on" } else { "off" },
            m.pi.word().halves_text(),
            m.ebr.halves_text(),
            m.ubr.halves_text(),
        );
        Reply::ok(
            text,
            json!({
                "pc": format!("{:06o}", m.pc.get()),
                "flags": format!("{:06o}", m.flags.0),
                "running": self.running,
                "stopped": self.stopped,
                "instructions": m.count,
                "idle_skipped": self.skipped,
                "ac": acs,
                "paging": m.paging,
            }),
        )
    }

    fn examine(&self, space: Space, a: u32, b: u32) -> Reply {
        let mut text = String::new();
        let mut words = Vec::new();
        for v in a..=b {
            let (shown, w) = match self.physical(space, v) {
                // A virtual address below 20 is an AC.
                Some(_) if space != Space::Phys && v < 0o20 => {
                    ("ac".into(), Some(self.m.ac_get(v as usize, false)))
                }
                Some(p) => (format!("{p:o}"), self.m.phys_read(p).ok()),
                None => ("-".into(), None),
            };
            let wt = w.map_or("no access".to_string(), |w| w.halves_text());
            if space == Space::Phys {
                text += &format!("{v:06o}/ {wt}\n");
            } else {
                text += &format!("{v:06o} ({shown})/ {wt}\n");
            }
            words.push(json!({
                "addr": format!("{v:o}"),
                "phys": shown,
                "word": w.map(|w| format!("{:012o}", w.get())),
            }));
        }
        Reply::ok(text, json!({"words": words}))
    }

    fn show_devices(&self) -> Reply {
        let mut text = String::new();
        let mut list = Vec::new();
        for (k, name) in [(0, "disk"), (1, "tape")] {
            for u in 0..8 {
                if let Some((p, ro)) = &self.images[k][u] {
                    text += &format!(
                        "{name} {u}: {}{}\n",
                        p.display(),
                        if *ro { " (ro)" } else { "" }
                    );
                    list.push(json!({"kind": name, "unit": u, "image": p, "read_only": ro}));
                }
            }
        }
        if text.is_empty() {
            text = "no images\n".into();
        }
        Reply::ok(text, json!({"units": list}))
    }

    /// Answer the expects that have their text or their limit.
    fn check_expects(&mut self) {
        let out = &self.m.console.output;
        let count = self.m.count;
        self.expects.retain(|e| {
            let from = e.from.min(out.len());
            if let Some(end) = find(&out[from..], &e.text) {
                let _ = e
                    .reply
                    .send((Reply::ok("", json!({"next": from + end})), from + end));
                false
            } else if count >= e.until {
                let _ = e
                    .reply
                    .send((Reply::err("expect: the text did not come"), e.from));
                false
            } else {
                true
            }
        });
    }

    /// Send new CTY output to the subscribers.
    fn distribute(&mut self) {
        let out = &self.m.console.output;
        if out.len() > self.sent {
            let b = out[self.sent..].to_vec();
            self.sent = out.len();
            self.subscribers
                .retain(|s| s.send(Out::Cty(b.clone())).is_ok());
        }
    }

    fn request(&mut self, r: Request) {
        match r {
            Request::Command { cmd, from, reply } => self.command(cmd, from, reply),
            Request::Keys(k) => {
                if !self.replaying() {
                    let _ = self.input(Input::Cty(k));
                }
            }
            Request::Subscribe(s) => {
                // The output so far first: a subscriber that comes late
                // (the terminal) loses nothing.
                let old = &self.m.console.output[..self.sent];
                if old.is_empty() || s.send(Out::Cty(old.to_vec())).is_ok() {
                    self.subscribers.push(s);
                }
            }
            Request::Units(s) => {
                let _ = s.send(self.units());
            }
        }
    }

    /// The count of the next event that must stop a batch.
    /// Start a script. Its first `expect` looks at the CTY output that
    /// comes after this moment.
    pub fn set_script(&mut self, s: Script, startup: bool) {
        self.script = Some(ScriptRun {
            s,
            from: self.m.console.output.len(),
            at: self.m.count,
            wait: None,
            startup,
        });
    }

    /// `script run FILE`: load a script while the machine runs.
    fn load_script(&mut self, path: &str) -> Result<String, String> {
        if let Some(r) = &self.script {
            return Err(format!("{} runs; give \"script stop\" first", r.s.name));
        }
        let s = Script::load(Path::new(path))?;
        if s.steps
            .iter()
            .any(|(_, c)| matches!(c, Command::ScriptRun(_)))
        {
            return Err(format!("{path}: a script cannot start a script"));
        }
        let t = format!("{}: {} steps", s.name, s.steps.len());
        self.set_script(s, false);
        Ok(t)
    }

    fn script_fail(&mut self, line: usize, e: &str) {
        let Some(r) = self.script.take() else { return };
        if r.startup {
            self.notice(format!("neoks10: {}:{line}: {e}", r.s.name));
            self.exit = 1;
            self.quit = true;
        } else {
            self.notice(format!(
                "neoks10: {}:{line}: {e}; the script stops",
                r.s.name
            ));
        }
    }

    /// Do the steps of the script that are due.
    fn script_tick(&mut self) {
        loop {
            let Some(r) = &mut self.script else { return };
            if let Some((_, until, line)) = r.wait {
                if self.m.count >= until {
                    self.script_fail(line, "expect: the text did not come");
                } else if !self.running {
                    self.script_fail(line, "expect: the processor stopped");
                }
                return;
            }
            let Some((line, cmd)) = r.s.steps.front().cloned() else {
                let name = r.s.name.clone();
                self.script = None;
                self.notice(format!("neoks10: {name}: done"));
                return;
            };
            match cmd {
                Command::Expect { text, steps } => {
                    let from = r.from.min(self.m.console.output.len());
                    r.s.steps.pop_front();
                    if let Some(end) = find(&self.m.console.output[from..], &text) {
                        r.from = from + end;
                        r.at = self.m.count;
                    } else {
                        let until = self.m.count.saturating_add(steps.unwrap_or(EXPECT_STEPS));
                        r.wait = Some((text, until, line));
                    }
                }
                Command::CtySend(b) => {
                    if self.m.count < r.at + ks10::console::ANSWER_DELAY {
                        return;
                    }
                    r.s.steps.pop_front();
                    r.at = self.m.count;
                    if let Err(e) = self.input(Input::Cty(b)) {
                        self.script_fail(line, &e);
                        return;
                    }
                }
                cmd => {
                    r.s.steps.pop_front();
                    r.at = self.m.count;
                    let (tx, _rx) = std::sync::mpsc::channel();
                    if let Some((rep, _)) = self.command_now(cmd, 0, &tx)
                        && !rep.ok
                    {
                        self.script_fail(line, &rep.text);
                        return;
                    }
                    if self.quit {
                        return;
                    }
                }
            }
        }
    }

    /// After an instruction that printed: does the waiting expect match?
    fn script_match(&mut self, before: usize) -> bool {
        let out = &self.m.console.output;
        let Some(r) = &mut self.script else {
            return false;
        };
        let Some((text, _, _)) = &r.wait else {
            return false;
        };
        let from = r.from.max(before.saturating_sub(text.len())).min(out.len());
        match find(&out[from..], text) {
            Some(end) => {
                r.from = from + end;
                r.at = self.m.count;
                r.wait = None;
                true
            }
            None => false,
        }
    }

    fn batch_end(&self) -> u64 {
        let mut end = self.m.count.saturating_add(BATCH);
        if let Some(r) = &self.script {
            match (&r.wait, r.s.steps.front()) {
                (Some((_, until, _)), _) => end = end.min(*until),
                (None, Some((_, Command::CtySend(_)))) => {
                    end = end.min(r.at + ks10::console::ANSWER_DELAY);
                }
                _ => {}
            }
        }
        if let Some(r) = self.replay.front() {
            end = end.min(r.count);
        }
        for e in &self.expects {
            end = end.min(e.until);
        }
        end
    }

    /// The main loop: requests, recorded inputs, instructions. Returns when
    /// a `quit` comes, when all senders are gone, or at the end of a replay
    /// with the processor stopped.
    pub fn serve(&mut self, rx: &Receiver<Request>) {
        let mut connected = true;
        while !self.quit {
            // Requests.
            loop {
                let r = if self.running || !connected {
                    match rx.try_recv() {
                        Ok(r) => Some(r),
                        Err(TryRecvError::Empty) => None,
                        Err(TryRecvError::Disconnected) => {
                            connected = false;
                            None
                        }
                    }
                } else {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(r) => Some(r),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => {
                            connected = false;
                            None
                        }
                    }
                };
                let Some(r) = r else { break };
                self.request(r);
                if self.quit {
                    return;
                }
            }
            // Recorded inputs that are due.
            while self.replay.front().is_some_and(|r| r.count <= self.m.count) {
                let Some(s) = self.replay.pop_front() else {
                    break;
                };
                // An image that is not the recorded one gives another run:
                // the replay ends.
                if let Err(e) = check_image(&s.input) {
                    self.notice(format!("neoks10: replay: {e}"));
                    self.exit = 1;
                    self.quit = true;
                    return;
                }
                if let Err(e) = self.apply(s.input) {
                    self.notice(format!("neoks10: replay: {e}"));
                }
            }
            self.script_tick();
            if self.quit {
                return;
            }
            if !self.running {
                if self.script.is_some() {
                    // Commands of the script before a boot; a script that
                    // waits for a stopped processor has failed.
                    self.script_tick();
                    if self.quit {
                        return;
                    }
                }
                if !connected
                    && (self.replay.is_empty() || self.replaying() && self.stopped.is_some())
                {
                    return;
                }
                self.distribute();
                self.check_expects();
                continue;
            }
            let end = self.batch_end();
            let skipped = self.skipped;
            let watch = self.script.as_ref().is_some_and(|r| r.wait.is_some());
            while self.m.count < end {
                let before = self.m.console.output.len();
                let r = if self.opts.idle {
                    self.m.step_idle(end).map(|n| self.skipped += n)
                } else {
                    self.m.step()
                };
                if let Err(s) = r {
                    self.note_stop(s);
                    break;
                }
                if watch && self.m.console.output.len() != before && self.script_match(before) {
                    break;
                }
            }
            self.distribute();
            self.check_expects();
            let ns = |n: u64| Duration::from_nanos(n.saturating_mul(self.opts.ns_per_instruction));
            let due = match self.opts.pace {
                Pace::Fast => continue,
                Pace::Idle => Instant::now() + ns(self.skipped - skipped),
                Pace::Realtime => self.base.0 + ns(self.m.count - self.base.1),
            };
            // Wait for the wall clock. Requests come in during the wait.
            while self.running {
                let now = Instant::now();
                if due <= now {
                    break;
                }
                if !connected {
                    std::thread::sleep(due - now);
                    break;
                }
                match rx.recv_timeout(due - now) {
                    Ok(r) => {
                        self.request(r);
                        if self.quit {
                            return;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => connected = false,
                }
            }
        }
    }
}

/// The SHA-256 of a file, in hexadecimal.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut f = File::open(path).map_err(err)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(err)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// For an `Attach` of a replay: the image must have the SHA-256 of the
/// record, if the record has one.
fn check_image(i: &Input) -> Result<(), String> {
    let Input::Attach {
        path,
        sha256: Some(want),
        ..
    } = i
    else {
        return Ok(());
    };
    let have = sha256_file(path)?;
    if &have != want {
        return Err(format!(
            "{}: the image is not the recorded one (SHA-256 {have}, the record has {want}); a replay needs a copy of the images as at the start of the recorded run",
            path.display()
        ));
    }
    Ok(())
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Disk => "disk",
        Kind::Tape => "tape",
    }
}

/// `path` relative to the directory `base`, both absolute, by their
/// components (no access to the file system).
pub fn relative(path: &Path, base: &Path) -> PathBuf {
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut r = PathBuf::new();
    for _ in common..b.len() {
        r.push("..");
    }
    for c in &p[common..] {
        r.push(c);
    }
    r
}

fn absolute(p: &str) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| PathBuf::from(p))
}

fn changes_guest(c: &Command) -> bool {
    !matches!(
        c,
        Command::Help
            | Command::Cty
            | Command::Quit
            | Command::CpuShow
            | Command::MemExamine(..)
            | Command::CtyRead(_)
            | Command::Expect { .. }
            | Command::ShowDevices
            | Command::ScriptShow
    )
}

/// The end of the first match of `pat` in `hay`.
fn find(hay: &[u8], pat: &[u8]) -> Option<usize> {
    if pat.is_empty() {
        return Some(0);
    }
    hay.windows(pat.len())
        .position(|w| w.iter().zip(pat).all(|(a, b)| a & 0o177 == *b))
        .map(|k| k + pat.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A subscriber that comes after the first CTY output gets it too
    // (the terminal subscribes after the control socket starts).
    #[test]
    fn late_subscriber_gets_old_output() {
        let mut e = Engine::new(options(None, None)).unwrap();
        e.m.console.output.extend_from_slice(b"\r\n DSKDMP");
        e.distribute();
        let (tx, rx) = std::sync::mpsc::channel();
        e.request(Request::Subscribe(tx));
        assert_eq!(rx.try_recv(), Ok(Out::Cty(b"\r\n DSKDMP".to_vec())));
        e.m.console.output.extend_from_slice(b"\r\n");
        e.distribute();
        assert_eq!(rx.try_recv(), Ok(Out::Cty(b"\r\n".to_vec())));
    }

    #[test]
    fn relative_paths() {
        let r = |a: &str, b: &str| relative(Path::new(a), Path::new(b));
        assert_eq!(r("/a/b/c.tape", "/a/b"), PathBuf::from("c.tape"));
        assert_eq!(r("/a/x/c.tape", "/a/b"), PathBuf::from("../x/c.tape"));
        assert_eq!(r("/c.tape", "/a/b"), PathBuf::from("../../c.tape"));
    }

    fn options(record: Option<PathBuf>, replay: Option<PathBuf>) -> Options {
        Options {
            ns_per_instruction: 1000,
            pace: Pace::Fast,
            idle: true,
            record,
            replay,
        }
    }

    // A run records its inputs; a replay of the record ends at the same
    // instruction count with the same memory. The guest is a loop
    // (JRST 1000); the keys go into the CTY input word 32 through the
    // 8080 model.
    #[test]
    fn record_and_replay() {
        let dir = std::env::temp_dir().join(format!("neoks10-rec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rec = dir.join("run.rec");
        let mut a = Engine::new(options(Some(rec.clone()), None)).unwrap();
        a.input(Input::Deposit {
            space: Space::Phys,
            addr: 0o1000,
            word: 0o254000_001000,
        })
        .unwrap();
        a.input(Input::Start {
            space: Space::Phys,
            addr: 0o1000,
        })
        .unwrap();
        for _ in 0..3 {
            for _ in 0..1777 {
                a.m.step().unwrap();
            }
            a.input(Input::Cty(b"x".to_vec())).unwrap();
        }
        for _ in 0..5000 {
            a.m.step().unwrap();
        }
        a.input(Input::Halt).unwrap();
        let (count, w32) = (a.m.count, a.m.mem[0o32]);
        assert_ne!(w32, W36::ZERO, "the 8080 gave a key");
        drop(a);
        let mut b = Engine::new(options(None, Some(rec))).unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<Request>();
        drop(tx);
        b.serve(&rx);
        assert_eq!(b.m.count, count);
        assert_eq!(b.m.mem[0o32], w32);
        assert!(!b.running);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // FIPS 180-2, appendix B.1: SHA-256 of "abc".
    #[test]
    fn sha256_of_abc() {
        let dir = std::env::temp_dir().join(format!("neoks10-sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("abc");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(
            sha256_file(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The record has the SHA-256 of a mounted tape. A replay with the same
    // image mounts it; a replay after a change of one byte ends with exit
    // status 1 and does not mount it.
    #[test]
    fn replay_checks_the_image() {
        let dir = std::env::temp_dir().join(format!("neoks10-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rec = dir.join("run.rec");
        let tape = dir.join("t.tape");
        std::fs::write(&tape, [4, 0, 0, 0, 1, 2, 3, 4, 4, 0, 0, 0]).unwrap();
        let mut a = Engine::new(options(Some(rec.clone()), None)).unwrap();
        a.input(Input::Attach {
            kind: Kind::Tape,
            unit: 0,
            path: tape.clone(),
            read_only: true,
            sha256: None,
        })
        .unwrap();
        a.input(Input::Quit).unwrap();
        drop(a);
        let text = std::fs::read_to_string(&rec).unwrap();
        assert!(text.contains(&sha256_file(&tape).unwrap()), "{text}");
        let replay = || {
            let mut b = Engine::new(options(None, Some(rec.clone()))).unwrap();
            let (tx, rx) = std::sync::mpsc::channel::<Request>();
            drop(tx);
            b.serve(&rx);
            (b.exit, b.images[1][0].is_some())
        };
        assert_eq!(replay(), (0, true));
        std::fs::write(&tape, [4, 0, 0, 0, 1, 2, 3, 5, 4, 0, 0, 0]).unwrap();
        assert_eq!(replay(), (1, false));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
