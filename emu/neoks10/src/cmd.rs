//! The command table of the operator console (docs/emulator.md, "Operator
//! console"). The parser, the completion, `help` and `docs/console.md`
//! come from this table.
//!
//! Syntax: a noun, a unit if the noun has units, a verb, then arguments.
//! Optional arguments come last. Guest addresses and words are octal.

use std::path::Path;

/// One element of a command pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    /// A fixed word.
    Lit(&'static str),
    /// A unit number of a kind.
    Unit(Kind),
    /// An optional unit number at the end (default 0).
    OptUnit(Kind),
    /// A file path; the completion offers files with these extensions.
    File(&'static [&'static str]),
    /// An address, or a range `A..B`, with an optional space prefix.
    Range,
    /// An address with an optional space prefix.
    Addr,
    /// A 36-bit word: octal, or `LH,,RH`.
    Word,
    /// One word of text; quotes keep spaces.
    Text,
    /// An optional decimal number at the end (default 1).
    OptCount,
    /// An optional flag.
    Flag(&'static str),
    /// An optional flag with a decimal number.
    Num(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Disk,
    Tape,
    /// A DZ11 line.
    Line,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Disk => "disk",
            Kind::Tape => "tape",
            Kind::Line => "dz line",
        }
    }
}

/// The commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    Help,
    Cty,
    Quit,
    CpuShow,
    CpuHalt,
    CpuContinue,
    CpuStep,
    CpuStart,
    BootDisk,
    BootTape,
    DiskAttach,
    DiskDetach,
    TapeMount,
    TapeUnmount,
    MemExamine,
    MemDeposit,
    CtySend,
    Send,
    CtyRead,
    Expect,
    ShowDevices,
    RecordMark,
    ScriptRun,
    ScriptStop,
    ScriptShow,
    DzShow,
    DzListen,
    DzClose,
    DzDisconnect,
}

pub struct Spec {
    pub id: Id,
    pub toks: &'static [Tok],
    pub help: &'static str,
    /// The command needs a stopped processor.
    pub halted: bool,
}

use Tok::*;

const DISK_FILES: &[&str] = &["dsk"];
const TAPE_FILES: &[&str] = &["tape", "tap"];
const SCRIPT_FILES: &[&str] = &["script"];

pub const SPECS: &[Spec] = &[
    Spec {
        id: Id::Help,
        toks: &[Lit("help")],
        help: "List the commands",
        halted: false,
    },
    Spec {
        id: Id::Cty,
        toks: &[Lit("cty")],
        help: "Return to the CTY (an empty line does the same)",
        halted: false,
    },
    Spec {
        id: Id::Quit,
        toks: &[Lit("quit")],
        help: "End the program",
        halted: false,
    },
    Spec {
        id: Id::CpuShow,
        toks: &[Lit("cpu"), Lit("show")],
        help: "Show the processor state",
        halted: false,
    },
    Spec {
        id: Id::CpuHalt,
        toks: &[Lit("cpu"), Lit("halt")],
        help: "Stop the processor (8080 HA)",
        halted: false,
    },
    Spec {
        id: Id::CpuContinue,
        toks: &[Lit("cpu"), Lit("continue")],
        help: "Continue the processor (8080 CO)",
        halted: false,
    },
    Spec {
        id: Id::CpuStep,
        toks: &[Lit("cpu"), Lit("step"), OptCount],
        help: "Do N instructions (8080 SI)",
        halted: true,
    },
    Spec {
        id: Id::CpuStart,
        toks: &[Lit("cpu"), Lit("start"), Addr],
        help: "Start at an address (8080 ST)",
        halted: true,
    },
    Spec {
        id: Id::BootDisk,
        toks: &[Lit("boot"), Lit("disk"), OptUnit(Kind::Disk)],
        help: "Boot from a disk unit (8080 BT)",
        halted: false,
    },
    Spec {
        id: Id::BootTape,
        toks: &[Lit("boot"), Lit("tape"), OptUnit(Kind::Tape)],
        help: "Boot from a tape transport (8080 MT)",
        halted: false,
    },
    Spec {
        id: Id::DiskAttach,
        toks: &[
            Lit("disk"),
            Unit(Kind::Disk),
            Lit("attach"),
            File(DISK_FILES),
            Flag("--ro"),
        ],
        help: "Attach a disk image to a unit",
        halted: false,
    },
    Spec {
        id: Id::DiskDetach,
        toks: &[Lit("disk"), Unit(Kind::Disk), Lit("detach")],
        help: "Detach the image of a disk unit",
        halted: false,
    },
    Spec {
        id: Id::TapeMount,
        toks: &[
            Lit("tape"),
            Unit(Kind::Tape),
            Lit("mount"),
            File(TAPE_FILES),
            Flag("--ro"),
        ],
        help: "Mount a tape image at the load point",
        halted: false,
    },
    Spec {
        id: Id::TapeUnmount,
        toks: &[Lit("tape"), Unit(Kind::Tape), Lit("unmount")],
        help: "Remove the tape of a transport",
        halted: false,
    },
    Spec {
        id: Id::MemExamine,
        toks: &[Lit("mem"), Lit("examine"), Range],
        help: "Show memory words (8080 EM)",
        halted: false,
    },
    Spec {
        id: Id::MemDeposit,
        toks: &[Lit("mem"), Lit("deposit"), Addr, Word],
        help: "Write a memory word (8080 DM)",
        halted: true,
    },
    Spec {
        id: Id::CtySend,
        toks: &[Lit("cty"), Lit("send"), Text],
        help: "Type text on the CTY (\\r, \\n, \\e, \\xHH)",
        halted: false,
    },
    Spec {
        id: Id::Send,
        toks: &[Lit("send"), Text],
        help: "Type text on the CTY (as cty send)",
        halted: false,
    },
    Spec {
        id: Id::CtyRead,
        toks: &[Lit("cty"), Lit("read"), Num("--from")],
        help: "The CTY output from byte N",
        halted: false,
    },
    Spec {
        id: Id::Expect,
        toks: &[Lit("expect"), Text, Num("--steps")],
        help: "Wait for text in the CTY output, at most N instructions",
        halted: false,
    },
    Spec {
        id: Id::ShowDevices,
        toks: &[Lit("show"), Lit("devices")],
        help: "Show the units and their images",
        halted: false,
    },
    Spec {
        id: Id::RecordMark,
        toks: &[Lit("record"), Lit("mark"), Text],
        help: "Write a mark into the record file",
        halted: false,
    },
    Spec {
        id: Id::ScriptRun,
        toks: &[Lit("script"), Lit("run"), File(SCRIPT_FILES)],
        help: "Run a script file; the guest goes on",
        halted: false,
    },
    Spec {
        id: Id::ScriptStop,
        toks: &[Lit("script"), Lit("stop")],
        help: "Stop the running script",
        halted: false,
    },
    Spec {
        id: Id::ScriptShow,
        toks: &[Lit("script"), Lit("show")],
        help: "Show the step of the running script",
        halted: false,
    },
    Spec {
        id: Id::DzShow,
        toks: &[Lit("dz"), Lit("show")],
        help: "Show the DZ11 lines: listeners and connections",
        halted: false,
    },
    Spec {
        id: Id::DzListen,
        toks: &[
            Lit("dz"),
            Lit("line"),
            Unit(Kind::Line),
            Lit("listen"),
            Text,
        ],
        help: "Listen for Telnet on HOST:PORT for the line",
        halted: false,
    },
    Spec {
        id: Id::DzClose,
        toks: &[Lit("dz"), Lit("line"), Unit(Kind::Line), Lit("close")],
        help: "Stop the listener of the line; a connection stays",
        halted: false,
    },
    Spec {
        id: Id::DzDisconnect,
        toks: &[Lit("dz"), Lit("line"), Unit(Kind::Line), Lit("disconnect")],
        help: "End the connection of the line",
        halted: false,
    },
];

/// The nouns, with their help text for the completion menu.
pub const NOUNS: &[(&str, &str)] = &[
    ("boot", "Boot from a disk or a tape"),
    ("cpu", "The processor: show, halt, continue, step, start"),
    ("cty", "The console terminal"),
    ("disk", "The RP06 disk units"),
    ("dz", "The DZ11 lines: Telnet listeners and connections"),
    ("expect", "Wait for text in the CTY output"),
    ("help", "List the commands"),
    ("mem", "The memory: examine, deposit"),
    ("quit", "End the program"),
    ("record", "The record file"),
    ("script", "Run a script file while the machine runs"),
    ("send", "Type text on the CTY"),
    ("show", "Show the devices"),
    ("tape", "The TU45 tape transports"),
];

/// An address space of `mem` and `cpu start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Space {
    Phys,
    Exec,
    User,
}

pub const SPACES: &[(&str, &str)] = &[
    ("phys:", "Physical memory (the default)"),
    ("exec:", "Executive virtual address"),
    ("user:", "User virtual address"),
];

/// A parsed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Cty,
    Quit,
    CpuShow,
    CpuHalt,
    CpuContinue,
    CpuStep(u64),
    CpuStart(Space, u32),
    BootDisk(usize),
    BootTape(usize),
    DiskAttach {
        unit: usize,
        path: String,
        read_only: bool,
    },
    DiskDetach(usize),
    TapeMount {
        unit: usize,
        path: String,
        read_only: bool,
    },
    TapeUnmount(usize),
    MemExamine(Space, u32, u32),
    MemDeposit(Space, u32, u64),
    CtySend(Vec<u8>),
    CtyRead(Option<u64>),
    Expect {
        text: Vec<u8>,
        steps: Option<u64>,
    },
    ShowDevices,
    RecordMark(String),
    ScriptRun(String),
    ScriptStop,
    ScriptShow,
    DzShow,
    DzListen(usize, String),
    DzClose(usize),
    DzDisconnect(usize),
}

impl Command {
    /// The command needs a stopped processor.
    pub fn needs_halt(&self) -> bool {
        let id = match self {
            Command::CpuStep(_) => Id::CpuStep,
            Command::CpuStart(..) => Id::CpuStart,
            Command::MemDeposit(..) => Id::MemDeposit,
            _ => return false,
        };
        SPECS.iter().any(|s| s.id == id && s.halted)
    }
}

/// A value of an argument.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Val {
    Unit(usize),
    Path(String),
    Range(Space, u32, u32),
    Addr(Space, u32),
    Word(u64),
    Text(String),
    Count(u64),
    Flag(&'static str),
    Num(&'static str, u64),
}

/// Split a line into words. Double quotes keep spaces; inside quotes, \"
/// and \\ are a quote and a backslash. Other escapes stay for `unescape`.
pub fn words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quoted = false;
    let mut it = line.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            '\\' if quoted => match it.next() {
                Some('"') => cur.push('"'),
                Some(o) => {
                    cur.push('\\');
                    cur.push(o);
                }
                None => cur.push('\\'),
            },
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quoted {
        return Err("a quote is not closed".into());
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

/// Text with \r, \n, \t, \e, \\ and \xHH escapes, as bytes.
pub fn unescape(x: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut it = x.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            let mut b = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            continue;
        }
        match it.next() {
            Some('r') => out.push(b'\r'),
            Some('n') => out.push(b'\n'),
            Some('t') => out.push(b'\t'),
            Some('e') => out.push(0o33),
            Some('\\') => out.push(b'\\'),
            Some('x') => {
                let h: String = it.by_ref().take(2).collect();
                let v = u8::from_str_radix(&h, 16)
                    .map_err(|_| format!("\\x{h}: two hex digits expected"))?;
                out.push(v);
            }
            Some(o) => return Err(format!("\\{o}: unknown escape")),
            None => return Err("a backslash at the end".into()),
        }
    }
    Ok(out)
}

fn octal(s: &str, max: u64) -> Option<u64> {
    let v = u64::from_str_radix(s, 8).ok()?;
    (v <= max).then_some(v)
}

fn addr(s: &str) -> Option<(Space, u32)> {
    let (space, rest) = match s.split_once(':') {
        Some(("phys", r)) => (Space::Phys, r),
        Some(("exec", r)) => (Space::Exec, r),
        Some(("user", r)) => (Space::User, r),
        Some(_) => return None,
        None => (Space::Phys, s),
    };
    let max = if space == Space::Phys {
        0o3777777
    } else {
        0o777777
    };
    Some((space, octal(rest, max)? as u32))
}

fn word(s: &str) -> Option<u64> {
    match s.split_once(",,") {
        Some((l, r)) => Some((octal(l, 0o777777)? << 18) | octal(r, 0o777777)?),
        None => octal(s, 0o777777_777777),
    }
}

fn unit(s: &str) -> Option<usize> {
    s.parse::<usize>().ok().filter(|&n| n < 8)
}

/// What an argument is called in help and errors.
fn tok_name(t: Tok) -> String {
    match t {
        Lit(w) => w.to_string(),
        Unit(_) => "UNIT".into(),
        OptUnit(_) => "[UNIT]".into(),
        File(_) => "FILE".into(),
        Range => "ADDR[..ADDR]".into(),
        Addr => "ADDR".into(),
        Word => "WORD".into(),
        Text => "TEXT".into(),
        OptCount => "[N]".into(),
        Flag(f) => format!("[{f}]"),
        Num(f) => format!("[{f} N]"),
    }
}

/// The usage line of a command.
pub fn usage(s: &Spec) -> String {
    s.toks
        .iter()
        .map(|&t| tok_name(t))
        .collect::<Vec<_>>()
        .join(" ")
}

fn optional(t: Tok) -> bool {
    matches!(t, OptUnit(_) | OptCount | Flag(_) | Num(_))
}

/// Match the words against one pattern. On failure: the number of
/// words that matched, and the error.
fn match_spec(s: &Spec, w: &[String]) -> Result<Vec<Val>, (usize, String)> {
    let mut vals = Vec::new();
    let mut i = 0;
    let mut k = 0;
    let fixed: Vec<Tok> = s.toks.iter().copied().filter(|&t| !optional(t)).collect();
    let opts: Vec<Tok> = s.toks.iter().copied().filter(|&t| optional(t)).collect();
    for &t in &fixed {
        let Some(x) = w.get(i) else {
            return Err((
                i,
                format!("{} expected after \"{}\"", tok_name(t), w.join(" ")),
            ));
        };
        let v = match t {
            Lit(l) if x == l => None,
            Lit(l) => return Err((i, format!("\"{x}\": \"{l}\" expected"))),
            Unit(_) => Some(Val::Unit(
                unit(x).ok_or((i, format!("\"{x}\": a unit 0 to 7 expected")))?,
            )),
            File(_) => Some(Val::Path(x.clone())),
            Range => {
                let (a, b) = match x.split_once("..") {
                    Some((a, b)) => {
                        let (sa, a) =
                            addr(a).ok_or((i, format!("\"{x}\": an octal address expected")))?;
                        let (_, b) =
                            addr(b).ok_or((i, format!("\"{x}\": an octal address expected")))?;
                        ((sa, a), b)
                    }
                    None => {
                        let a =
                            addr(x).ok_or((i, format!("\"{x}\": an octal address expected")))?;
                        (a, a.1)
                    }
                };
                if b < a.1 {
                    return Err((i, format!("\"{x}\": the range ends before it starts")));
                }
                Some(Val::Range(a.0, a.1, b))
            }
            Addr => {
                let (sp, a) = addr(x).ok_or((i, format!("\"{x}\": an octal address expected")))?;
                Some(Val::Addr(sp, a))
            }
            Word => Some(Val::Word(
                word(x).ok_or((i, format!("\"{x}\": an octal word or LH,,RH expected")))?,
            )),
            Text => Some(Val::Text(x.clone())),
            _ => None,
        };
        vals.extend(v);
        i += 1;
        k += 1;
    }
    let _ = k;
    let mut used = Vec::new();
    while let Some(x) = w.get(i) {
        let hit = opts
            .iter()
            .copied()
            .find(|&t| !used.contains(&t) && opt_matches(t, x));
        let Some(t) = hit else {
            return Err((i, format!("\"{x}\": unexpected; usage: {}", usage(s))));
        };
        used.push(t);
        match t {
            OptUnit(_) => vals.push(Val::Unit(unit(x).unwrap_or(0))),
            OptCount => vals.push(Val::Count(x.parse().unwrap_or(1))),
            Flag(f) => vals.push(Val::Flag(f)),
            Num(f) => {
                i += 1;
                let n = w
                    .get(i)
                    .and_then(|n| n.parse::<u64>().ok())
                    .ok_or((i, format!("{f}: a decimal number expected")))?;
                vals.push(Val::Num(f, n));
            }
            _ => {}
        }
        i += 1;
    }
    Ok(vals)
}

fn opt_matches(t: Tok, x: &str) -> bool {
    match t {
        OptUnit(_) => unit(x).is_some(),
        OptCount => x.parse::<u64>().is_ok(),
        Flag(f) | Num(f) => x == f,
        _ => false,
    }
}

/// Edit distance, for "did you mean".
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

fn nearest<'a>(x: &str, options: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    options
        .map(|o| (distance(x, o), o))
        .filter(|&(d, o)| d <= 2.max(o.len() / 3))
        .min()
        .map(|(_, o)| o)
}

/// Parse a command line. An empty line is `None`.
pub fn parse(line: &str) -> Result<Option<Command>, String> {
    let w = words(line)?;
    if w.is_empty() {
        return Ok(None);
    }
    if !NOUNS.iter().any(|&(n, _)| n == w[0]) {
        let hint = nearest(&w[0], NOUNS.iter().map(|&(n, _)| n))
            .map(|n| format!("; did you mean \"{n}\"?"))
            .unwrap_or_default();
        return Err(format!("unknown noun \"{}\"{hint}", w[0]));
    }
    let mut best: Option<(usize, String)> = None;
    for s in SPECS {
        match match_spec(s, &w) {
            Ok(vals) => return build(s.id, vals).map(Some),
            Err((n, e)) => {
                if best.as_ref().is_none_or(|b| n > b.0) {
                    best = Some((n, e));
                }
            }
        }
    }
    let (n, e) = best.unwrap_or((0, "unknown command".into()));
    // A wrong verb: name the nearest one of the noun.
    if n >= 1 && n < w.len() {
        let verbs = SPECS.iter().filter_map(|s| {
            let lits: Vec<&str> = s
                .toks
                .iter()
                .filter_map(|t| if let Lit(l) = t { Some(*l) } else { None })
                .collect();
            (lits.first() == Some(&w[0].as_str()) && lits.len() > 1).then(|| lits[1])
        });
        let verbs: Vec<&str> = verbs.collect();
        if !verbs.contains(&w[n].as_str())
            && let Some(v) = nearest(&w[n], verbs.iter().copied())
        {
            return Err(format!("{e}; did you mean \"{v}\"?"));
        }
    }
    Err(e)
}

fn build(id: Id, v: Vec<Val>) -> Result<Command, String> {
    let unit = || {
        v.iter()
            .find_map(|x| if let Val::Unit(u) = x { Some(*u) } else { None })
            .unwrap_or(0)
    };
    let path = || {
        v.iter()
            .find_map(|x| {
                if let Val::Path(p) = x {
                    Some(p.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    };
    let flag = |f: &str| v.iter().any(|x| matches!(x, Val::Flag(g) if *g == f));
    let num = |f: &str| {
        v.iter().find_map(|x| {
            if let Val::Num(g, n) = x {
                (*g == f).then_some(*n)
            } else {
                None
            }
        })
    };
    let text = || {
        v.iter()
            .find_map(|x| {
                if let Val::Text(t) = x {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    };
    let addr = || {
        v.iter()
            .find_map(|x| {
                if let Val::Addr(s, a) = x {
                    Some((*s, *a))
                } else {
                    None
                }
            })
            .unwrap_or((Space::Phys, 0))
    };
    Ok(match id {
        Id::Help => Command::Help,
        Id::Cty => Command::Cty,
        Id::Quit => Command::Quit,
        Id::CpuShow => Command::CpuShow,
        Id::CpuHalt => Command::CpuHalt,
        Id::CpuContinue => Command::CpuContinue,
        Id::CpuStep => Command::CpuStep(
            v.iter()
                .find_map(|x| {
                    if let Val::Count(n) = x {
                        Some(*n)
                    } else {
                        None
                    }
                })
                .unwrap_or(1),
        ),
        Id::CpuStart => {
            let (s, a) = addr();
            Command::CpuStart(s, a)
        }
        Id::BootDisk => Command::BootDisk(unit()),
        Id::BootTape => Command::BootTape(unit()),
        Id::DiskAttach => Command::DiskAttach {
            unit: unit(),
            path: path(),
            read_only: flag("--ro"),
        },
        Id::DiskDetach => Command::DiskDetach(unit()),
        Id::TapeMount => Command::TapeMount {
            unit: unit(),
            path: path(),
            read_only: flag("--ro"),
        },
        Id::TapeUnmount => Command::TapeUnmount(unit()),
        Id::MemExamine => {
            let (s, a, b) = v
                .iter()
                .find_map(|x| {
                    if let Val::Range(s, a, b) = x {
                        Some((*s, *a, *b))
                    } else {
                        None
                    }
                })
                .unwrap_or((Space::Phys, 0, 0));
            if b - a >= 0o10000 {
                return Err("at most 10000 words at a time".into());
            }
            Command::MemExamine(s, a, b)
        }
        Id::MemDeposit => {
            let (s, a) = addr();
            let w = v
                .iter()
                .find_map(|x| if let Val::Word(w) = x { Some(*w) } else { None })
                .unwrap_or(0);
            Command::MemDeposit(s, a, w)
        }
        Id::CtySend | Id::Send => Command::CtySend(unescape(&text())?),
        Id::CtyRead => Command::CtyRead(num("--from")),
        Id::Expect => Command::Expect {
            text: unescape(&text())?,
            steps: num("--steps"),
        },
        Id::ShowDevices => Command::ShowDevices,
        Id::RecordMark => Command::RecordMark(text()),
        Id::ScriptRun => Command::ScriptRun(path()),
        Id::ScriptStop => Command::ScriptStop,
        Id::ScriptShow => Command::ScriptShow,
        Id::DzShow => Command::DzShow,
        Id::DzListen => Command::DzListen(unit(), text()),
        Id::DzClose => Command::DzClose(unit()),
        Id::DzDisconnect => Command::DzDisconnect(unit()),
    })
}

/// The help text: one line for each command.
pub fn help() -> String {
    let rows: Vec<(String, &str)> = SPECS.iter().map(|s| (usage(s), s.help)).collect();
    let width = rows.iter().map(|r| r.0.len()).max().unwrap_or(0);
    rows.iter()
        .map(|(u, h)| format!("{u:width$}  {h}\n"))
        .collect()
}

/// docs/console.md.
pub fn markdown() -> String {
    let mut s = String::from(
        "# Operator console commands\n\n\
         Generated by `neoks10 help --markdown` from `emu/neoks10/src/cmd.rs`. \
         Do not edit. The design is in `docs/emulator.md`, \"Operator console\".\n\n\
         | Command | Function | Needs a halt |\n|---|---|---|\n",
    );
    for sp in SPECS {
        s += &format!(
            "| `{}` | {} | {} |\n",
            usage(sp).replace('|', "\\|"),
            sp.help.replace('|', "\\|"),
            if sp.halted { "yes" } else { "no" }
        );
    }
    s += "\nADDR is octal, with an optional space: `phys:` (the default), \
          `exec:` or `user:`. WORD is octal or `LH,,RH`. UNIT is 0 to 7. \
          TEXT is one word, or text in double quotes.\n";
    s
}

/// The units for the completion: number and a description.
#[derive(Clone, Debug, Default)]
pub struct Units {
    pub disk: Vec<(usize, String)>,
    pub tape: Vec<(usize, String)>,
    pub line: Vec<(usize, String)>,
}

/// A completion candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cand {
    pub value: String,
    pub help: String,
    /// Add a space after the value.
    pub space: bool,
}

/// The candidates for the word at byte `pos` of `line`, and the byte
/// where that word starts.
pub fn complete(line: &str, pos: usize, units: &Units) -> (usize, Vec<Cand>) {
    let head = &line[..pos.min(line.len())];
    let start = head.rfind(char::is_whitespace).map_or(0, |k| k + 1);
    let partial = &head[start..];
    let Ok(done) = words(&head[..start]) else {
        return (start, Vec::new());
    };
    let mut out: Vec<Cand> = Vec::new();
    let mut push = |c: Cand| {
        if c.value.starts_with(partial) && !out.iter().any(|o| o.value == c.value) {
            out.push(c);
        }
    };
    if done.is_empty() {
        for &(n, h) in NOUNS {
            push(Cand {
                value: n.into(),
                help: h.into(),
                space: true,
            });
        }
        return (start, out);
    }
    for s in SPECS {
        for t in next_toks(s, &done) {
            match t {
                Lit(l) => {
                    let last = !s
                        .toks
                        .iter()
                        .skip_while(|&&x| x != Lit(l))
                        .skip(1)
                        .any(|&x| matches!(x, Lit(_)));
                    let help = if last {
                        s.help.to_string()
                    } else {
                        String::new()
                    };
                    push(Cand {
                        value: l.into(),
                        help,
                        space: true,
                    });
                }
                Unit(k) | OptUnit(k) => {
                    let list = match k {
                        Kind::Disk => &units.disk,
                        Kind::Tape => &units.tape,
                        Kind::Line => &units.line,
                    };
                    for (n, d) in list {
                        push(Cand {
                            value: n.to_string(),
                            help: format!("{} {n}: {d}", k.name()),
                            space: true,
                        });
                    }
                }
                Flag(f) | Num(f) => push(Cand {
                    value: f.into(),
                    help: tok_name(t),
                    space: true,
                }),
                File(exts) => {
                    for c in files(partial, exts) {
                        push(c);
                    }
                }
                Range | Addr => {
                    for &(p, h) in SPACES {
                        push(Cand {
                            value: p.into(),
                            help: h.into(),
                            space: false,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    (start, out)
}

/// The tokens of `s` that can come after the words `done`.
fn next_toks(s: &Spec, done: &[String]) -> Vec<Tok> {
    let fixed: Vec<Tok> = s.toks.iter().copied().filter(|&t| !optional(t)).collect();
    let opts: Vec<Tok> = s.toks.iter().copied().filter(|&t| optional(t)).collect();
    for (i, x) in done.iter().enumerate() {
        match fixed.get(i) {
            Some(Lit(l)) if x != l => return Vec::new(),
            Some(Unit(_)) if unit(x).is_none() => return Vec::new(),
            Some(_) => {}
            None => {
                // Among the optional words: offer the unused ones.
                let used = &done[fixed.len()..];
                if used
                    .last()
                    .is_some_and(|l| opts.iter().any(|&t| matches!(t, Num(f) if f == l)))
                {
                    return Vec::new();
                }
                return opts
                    .iter()
                    .copied()
                    .filter(|&t| !used.iter().any(|u| opt_matches(t, u)))
                    .collect();
            }
        }
    }
    match fixed.get(done.len()) {
        Some(&t) => vec![t],
        None => opts,
    }
}

/// File candidates for a partial path: directories, and files with one of
/// the extensions.
fn files(partial: &str, exts: &[&str]) -> Vec<Cand> {
    let (dir, prefix) = match partial.rfind('/') {
        Some(k) => (&partial[..=k], &partial[k + 1..]),
        None => ("", partial),
    };
    let read = if dir.is_empty() {
        Path::new(".")
    } else {
        Path::new(dir)
    };
    let Ok(rd) = std::fs::read_dir(read) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with(prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
            continue;
        }
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        let ext_ok = Path::new(&name)
            .extension()
            .is_some_and(|x| exts.iter().any(|&e| x == e));
        if is_dir {
            out.push(Cand {
                value: format!("{dir}{name}/"),
                help: "directory".into(),
                space: false,
            });
        } else if ext_ok {
            out.push(Cand {
                value: format!("{dir}{name}"),
                help: String::new(),
                space: true,
            });
        }
    }
    out.sort_by(|a, b| a.value.cmp(&b.value));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_commands() {
        assert_eq!(parse("  ").unwrap(), None);
        assert_eq!(
            parse("tape 2 mount \"my tapes/minsys.tape\" --ro").unwrap(),
            Some(Command::TapeMount {
                unit: 2,
                path: "my tapes/minsys.tape".into(),
                read_only: true
            })
        );
        assert_eq!(parse("boot disk").unwrap(), Some(Command::BootDisk(0)));
        assert_eq!(parse("boot tape 1").unwrap(), Some(Command::BootTape(1)));
        assert_eq!(
            parse("mem examine exec:1000..1003").unwrap(),
            Some(Command::MemExamine(Space::Exec, 0o1000, 0o1003))
        );
        assert_eq!(
            parse("mem deposit 1000 254000,,1000").unwrap(),
            Some(Command::MemDeposit(Space::Phys, 0o1000, 0o254000_001000))
        );
        assert_eq!(parse("cpu step").unwrap(), Some(Command::CpuStep(1)));
        assert_eq!(parse("cpu step 25").unwrap(), Some(Command::CpuStep(25)));
        assert_eq!(
            parse(r#"cty send "its\r""#).unwrap(),
            Some(Command::CtySend(b"its\r".to_vec()))
        );
        assert_eq!(
            parse(r#"expect "DSKDMP" --steps 1000"#).unwrap(),
            Some(Command::Expect {
                text: b"DSKDMP".to_vec(),
                steps: Some(1000)
            })
        );
        assert_eq!(
            parse("cty read --from 12").unwrap(),
            Some(Command::CtyRead(Some(12)))
        );
        assert_eq!(
            parse("script run boot.script").unwrap(),
            Some(Command::ScriptRun("boot.script".into()))
        );
    }

    #[test]
    fn errors() {
        assert_eq!(
            parse("tpe 0 mount x").unwrap_err(),
            "unknown noun \"tpe\"; did you mean \"tape\"?"
        );
        assert!(
            parse("tape 0 mout x")
                .unwrap_err()
                .ends_with("did you mean \"mount\"?")
        );
        assert!(parse("tape 9 mount x").unwrap_err().contains("unit 0 to 7"));
        assert!(parse("mem examine 1009").unwrap_err().contains("octal"));
        assert!(
            parse("mem examine 2000..1000")
                .unwrap_err()
                .contains("ends before")
        );
        assert!(parse("cty send \"abc").unwrap_err().contains("quote"));
        assert!(
            parse("tape 0 mount x --rw")
                .unwrap_err()
                .contains("unexpected")
        );
    }

    fn values(line: &str, units: &Units) -> Vec<String> {
        complete(line, line.len(), units)
            .1
            .into_iter()
            .map(|c| c.value)
            .collect()
    }

    #[test]
    fn completion() {
        let units = Units {
            disk: vec![(0, "rp0.dsk".into())],
            tape: vec![(0, "minsys.tape".into()), (2, "-".into())],
            line: vec![(5, "127.0.0.1:10009".into())],
        };
        assert_eq!(values("ta", &units), ["tape"]);
        assert_eq!(values("dz line ", &units), ["5"]);
        assert_eq!(
            values("dz line 5 ", &units),
            ["listen", "close", "disconnect"]
        );
        assert_eq!(
            parse("dz line 5 listen 127.0.0.1:10009").unwrap(),
            Some(Command::DzListen(5, "127.0.0.1:10009".into()))
        );
        assert_eq!(values("tape ", &units), ["0", "2"]);
        assert_eq!(values("tape 2 ", &units), ["mount", "unmount"]);
        assert_eq!(values("tape 2 m", &units), ["mount"]);
        assert_eq!(values("cpu s", &units), ["show", "step", "start"]);
        assert_eq!(values("boot ", &units), ["disk", "tape"]);
        assert_eq!(values("script ", &units), ["run", "stop", "show"]);
        assert_eq!(values("boot tape ", &units), ["0", "2"]);
        assert_eq!(values("mem examine ", &units), ["phys:", "exec:", "user:"]);
        assert_eq!(values("expect x ", &units), ["--steps"]);
        assert_eq!(values("expect x --steps ", &units), Vec::<String>::new());
        // The span starts at the word under the cursor.
        assert_eq!(complete("tape 2 mo", 9, &units).0, 7);
    }

    #[test]
    fn file_completion() {
        let dir = std::env::temp_dir().join(format!("neoks10-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.tape"), b"").unwrap();
        std::fs::write(dir.join("b.dsk"), b"").unwrap();
        let units = Units {
            tape: vec![(0, String::new())],
            ..Default::default()
        };
        let d = dir.to_string_lossy();
        let line = format!("tape 0 mount {d}/");
        let v = values(&line, &units);
        assert_eq!(v, [format!("{d}/a.tape"), format!("{d}/sub/")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn help_lists_all() {
        let h = help();
        assert_eq!(h.lines().count(), SPECS.len());
        assert!(h.contains("tape UNIT mount FILE [--ro]"));
        assert!(markdown().contains("| `mem deposit ADDR WORD` |"));
    }
}
