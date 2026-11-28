//! The program `neoks10` (docs/emulator.md, "Command line").

mod cmd;
mod config;
mod control;
mod dz;
mod engine;
mod script;
mod term;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::mpsc::channel;

use config::{BootDevice, Config};
use engine::{Engine, Input, Kind, Options, Out, Pace, Request};

const USAGE: &str = "\
usage: neoks10 run [CONFIG] [--control PATH | --no-control] [--record FILE]
                   [--replay FILE] [--script FILE] [--pace normal|fast|realtime]
                   [--no-idle] [--stats]
       neoks10 console [--control PATH]
       neoks10 ctl [--control PATH] [--json] COMMAND...
       neoks10 help [--markdown]

In `run`, Ctrl-] opens the operator prompt; `help` there lists the commands.
Pace: `normal` takes the clock of the KS10 from the wall clock (as SIMH
does) and cannot be recorded; `fast` does not wait; `realtime` keeps the
virtual time to the wall clock. `run` with a terminal or a control socket
uses `normal`; with --record it needs --pace fast or realtime. A run with
--script or --replay, or with neither a terminal nor a socket, uses
`fast`.
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
    /// `--pace`; else the default of the run.
    pace: Option<Pace>,
    idle: bool,
    control: Option<PathBuf>,
    no_control: bool,
    script: Option<PathBuf>,
    stats: bool,
}

fn run_args(args: &[String]) -> Result<RunArgs, String> {
    let mut r = RunArgs {
        config: None,
        record: None,
        replay: None,
        pace: None,
        idle: true,
        control: None,
        no_control: false,
        script: None,
        stats: false,
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
            "--pace" => {
                r.pace = Some(match it.next().map(String::as_str) {
                    Some("normal") => Pace::Normal,
                    Some("fast") => Pace::Fast,
                    Some("realtime") => Pace::Realtime,
                    _ => return Err("--pace needs normal, fast or realtime".into()),
                })
            }
            "--no-idle" => r.idle = false,
            "--stats" => r.stats = true,
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
    // A run that people or programs use through the terminal or the
    // socket takes the clock from the wall clock; a script or a replay
    // does not wait.
    let interactive = std::io::stdin().is_terminal() && a.replay.is_none();
    let pace = match a.pace {
        Some(p) => p,
        None if a.script.is_none()
            && a.replay.is_none()
            && (interactive || !a.no_control || cfg.dz11.is_some()) =>
        {
            // A record must be repeatable, and `normal` is not: the
            // choice of the other two is for the user.
            if a.record.is_some() {
                return Err(
                    "a run with a terminal or a socket and --record needs --pace fast or realtime"
                        .into(),
                );
            }
            Pace::Normal
        }
        None => Pace::Fast,
    };
    let mut e = Engine::new(Options {
        pace,
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
    // The DZ11 listeners send their requests through `tx`. A run that
    // takes requests ends only with `quit`.
    let takes_requests = interactive || !a.no_control || cfg.dz11.is_some();
    if takes_requests && !e.replaying() {
        e.set_requests(tx.clone());
        if let Some(d) = &cfg.dz11 {
            for (k, addr) in d.addresses()?.iter().enumerate() {
                eprintln!("neoks10: {}", e.dz_listen(k, addr)?);
            }
        }
    }
    // Set when the machine thread ends (a quit from the socket or a
    // script), so that the terminal leaves raw mode and returns.
    let ended = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = ended.clone();
    let machine = std::thread::spawn(move || {
        let start = std::time::Instant::now();
        e.serve(&rx);
        let time = start.elapsed();
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        (e.exit, e.counts(), time)
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
    let mut printer = None;
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
        printer = Some(std::thread::spawn(move || {
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
        }));
    }
    drop(tx);
    let joined = machine.join();
    // The engine and its subscribers end with the machine thread; the
    // printer then writes the rest of the CTY output and ends.
    if let Some(p) = printer {
        let _ = p.join();
    }
    if let Some(p) = &socket {
        control::remove(p);
    }
    let (exit, (count, skipped), time) =
        joined.map_err(|_| "the machine thread failed".to_string())?;
    if a.stats {
        eprintln!("neoks10: {}", stats(count, skipped, time));
    }
    Ok(exit)
}

/// The statistics of `run --stats`: the instructions that the processor
/// did one by one, the time of the machine thread, and their rate.
fn stats(count: u64, skipped: u64, time: std::time::Duration) -> String {
    let done = count - skipped;
    let s = time.as_secs_f64();
    let mips = if s > 0.0 { done as f64 / s / 1e6 } else { 0.0 };
    format!(
        "{done} instructions in {s:.3} s, {mips:.2} MIPS ({skipped} more as idle passes at once)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_count_only_the_instructions_done_one_by_one() {
        let t = std::time::Duration::from_millis(1500);
        assert_eq!(
            stats(3_000_500, 500, t),
            "3000000 instructions in 1.500 s, 2.00 MIPS (500 more as idle passes at once)"
        );
    }
}
