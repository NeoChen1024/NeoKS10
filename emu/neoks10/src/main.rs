//! The program `neoks10` (docs/emulator.md, "Command line").

mod cmd;
mod config;
mod control;
mod engine;
mod script;
mod term;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::mpsc::channel;

use config::{BootDevice, Config};
use engine::{Engine, Input, Kind, Options, Out, Request};

const USAGE: &str = "\
usage: neoks10 run [CONFIG] [--control PATH | --no-control] [--record FILE]
                   [--replay FILE] [--script FILE] [--realtime | --fast]
                   [--no-idle]
       neoks10 console [--control PATH]
       neoks10 ctl [--control PATH] [--json] COMMAND...
       neoks10 help [--markdown]

In `run`, Ctrl-\\ opens the operator prompt; `help` there lists the commands.
`run` on a terminal without --script keeps the virtual time to the wall
clock (--realtime); other runs go as fast as possible (--fast).
`run` listens on the control socket neoks10.sock in the directory of CONFIG
(or the current directory). `console` and `ctl` use NEOKS10_CONTROL, or
neoks10.sock in the current directory, if --control is not given.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("console") => control::console(&args[1..]),
        Some("ctl") => control::ctl(&args[1..]),
        Some("help") => {
            if args.get(1).is_some_and(|a| a == "--markdown") {
                print!("{}", cmd::markdown());
            } else {
                println!("{USAGE}\n\nOperator commands:\n{}", cmd::help());
            }
            Ok(0)
        }
        _ => {
            eprintln!("{USAGE}");
            Ok(2)
        }
    };
    match code {
        Ok(c) => std::process::exit(c),
        Err(e) => {
            eprintln!("neoks10: {e}");
            std::process::exit(2);
        }
    }
}

struct RunArgs {
    config: Option<PathBuf>,
    record: Option<PathBuf>,
    replay: Option<PathBuf>,
    /// `--realtime` or `--fast`; else the default of the run.
    realtime: Option<bool>,
    idle: bool,
    control: Option<PathBuf>,
    no_control: bool,
    script: Option<PathBuf>,
}

fn run_args(args: &[String]) -> Result<RunArgs, String> {
    let mut r = RunArgs {
        config: None,
        record: None,
        replay: None,
        realtime: None,
        idle: true,
        control: None,
        no_control: false,
        script: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{name} needs a file"))
        };
        match a.as_str() {
            "--record" => r.record = Some(value("--record")?),
            "--replay" => r.replay = Some(value("--replay")?),
            "--realtime" => r.realtime = Some(true),
            "--fast" => r.realtime = Some(false),
            "--no-idle" => r.idle = false,
            "--control" => r.control = Some(value("--control")?),
            "--no-control" => r.no_control = true,
            "--script" => r.script = Some(value("--script")?),
            "--trace" | "--cpu" => {
                return Err(format!("{a} is not in this version yet"));
            }
            x if x.starts_with('-') => return Err(format!("unknown option {x}\n{USAGE}")),
            x => {
                if r.config.is_some() {
                    return Err(format!("one configuration file only\n{USAGE}"));
                }
                r.config = Some(PathBuf::from(x));
            }
        }
    }
    if r.record.is_some() && r.replay.is_some() {
        return Err("--record and --replay together are not possible".into());
    }
    Ok(r)
}

fn run(args: &[String]) -> Result<i32, String> {
    let a = run_args(args)?;
    let cfg = match &a.config {
        Some(p) => Config::load(p)?,
        None => Config::default(),
    };
    // A person at the terminal needs the guest clock to follow the wall
    // clock; a script, a replay or a program does not.
    let interactive = std::io::stdin().is_terminal() && a.replay.is_none();
    let mut e = Engine::new(Options {
        ns_per_instruction: cfg.machine.time_per_instruction_ns,
        realtime: a.realtime.unwrap_or(interactive && a.script.is_none()),
        idle: a.idle,
        record: a.record,
        replay: a.replay,
    })?;
    // The configuration is an input too: a replay has it in its record.
    if !e.replaying() {
        for (kind, map) in [(Kind::Disk, &cfg.disk), (Kind::Tape, &cfg.tape)] {
            for (unit, img) in Config::units(map) {
                let path = std::path::absolute(cfg.path(&img.image)).map_err(|x| x.to_string())?;
                e.input(Input::Attach {
                    kind,
                    unit,
                    path,
                    read_only: img.read_only,
                    sha256: None,
                })?;
            }
        }
        if let Some(b) = cfg.boot {
            let kind = match b.device {
                BootDevice::Disk => Kind::Disk,
                BootDevice::Tape => Kind::Tape,
            };
            e.input(Input::Boot {
                kind,
                unit: usize::from(b.unit),
            })?;
        }
    }
    if let Some(p) = &a.script {
        if e.replaying() {
            return Err("--script and --replay together are not possible".into());
        }
        e.set_script(script::Script::load(p)?, true);
    }
    let (tx, rx) = channel::<Request>();
    // Set when the machine thread ends (a quit from the socket or a
    // script), so that the terminal leaves raw mode and returns.
    let ended = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = ended.clone();
    let machine = std::thread::spawn(move || {
        e.serve(&rx);
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        e.exit
    });
    // The control socket. While it listens, `run` ends only with `quit`.
    let socket = if a.no_control {
        None
    } else {
        let p = a
            .control
            .or_else(|| cfg.control.socket.as_ref().map(|s| cfg.path(s)))
            .unwrap_or_else(|| cfg.dir.join("neoks10.sock"));
        control::listen(&p, tx.clone())?;
        Some(p)
    };
    if interactive {
        term::run(tx.clone(), &ended);
        let (rtx, _rrx) = channel();
        let _ = tx.send(Request::Command {
            cmd: cmd::Command::Quit,
            from: 0,
            reply: rtx,
        });
    } else {
        // Without a terminal: the CTY output to stdout, until the
        // processor stops.
        let (otx, orx) = channel();
        let _ = tx.send(Request::Subscribe(otx));
        std::thread::spawn(move || {
            for o in orx {
                let mut so = std::io::stdout().lock();
                match o {
                    Out::Cty(b) => {
                        let b: Vec<u8> = b.iter().map(|c| c & 0o177).filter(|&c| c != 0).collect();
                        let _ = so.write_all(&b);
                    }
                    Out::Notice(t) => {
                        let _ = writeln!(so, "\n{t}");
                    }
                    Out::Flush => {}
                }
                let _ = so.flush();
            }
        });
    }
    drop(tx);
    let joined = machine.join();
    if let Some(p) = &socket {
        control::remove(p);
    }
    joined.map_err(|_| "the machine thread failed".to_string())
}
