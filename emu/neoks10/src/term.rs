//! The terminal of `neoks10 run` (docs/emulator.md, "Operator console"):
//! CTY mode and command mode.
//!
//! - CTY mode: raw mode; each key goes to the CTY, the CTY output goes to
//!   the terminal. The escape character Ctrl-] (035) opens the prompt.
//!   Ctrl-\ (034) opens it too: it is the key of the console mode of the
//!   8080 program, and the prompt has the functions of that mode.
//! - Command mode: the reedline prompt. The guest goes on; its CTY output
//!   waits until the prompt closes. An empty line, `cty`, Ctrl-C or Ctrl-D
//!   close the prompt.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crossterm::event::{Event, KeyEvent, KeyEventKind};
use reedline::{
    ColumnarMenu, Completer, CompletionResult, DefaultHinter, DefaultPrompt, DefaultPromptSegment,
    Emacs, FileBackedHistory, KeyCode, KeyModifiers, MenuBuilder, Reedline, ReedlineEvent,
    ReedlineMenu, Signal, Span, Suggestion, default_emacs_keybindings,
};

use crate::cmd::{self, Command, Units};
use crate::engine::{Out, Reply, Request};

/// The escape character of the operator prompt (docs/emulator.md, design
/// decision 3).
pub const ESCAPE: u8 = 0o35;

/// Ctrl-\: the 8080 program leaves user mode with it and gives its prompt
/// `KS10>` **[its]** (hcore.m80, `USER`, `CNBCK`). The high-level console
/// has no such mode, so this key opens the operator prompt too.
pub const CONSOLE_ESCAPE: u8 = 0o34;

/// The completer of the console: the command table and the units. The
/// console updates the units after each command.
pub struct Complete {
    pub units: Arc<Mutex<Units>>,
}

impl Completer for Complete {
    fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
        let units = self.units.lock().map(|u| u.clone()).unwrap_or_default();
        let (start, cands) = cmd::complete(line, pos, &units);
        let v: Vec<Suggestion> = cands
            .into_iter()
            .map(|c| Suggestion {
                value: c.value,
                description: (!c.help.is_empty()).then_some(c.help),
                span: Span { start, end: pos },
                append_whitespace: c.space,
                ..Default::default()
            })
            .collect();
        CompletionResult::fresh(v)
    }
}

/// A line editor with the command completion, a menu on Tab, and hints
/// from the history.
pub fn editor(units: Arc<Mutex<Units>>) -> Reedline {
    let menu = ColumnarMenu::default().with_name("completion_menu");
    let mut keys = default_emacs_keybindings();
    keys.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".into()),
            ReedlineEvent::MenuNext,
        ]),
    );
    let history = FileBackedHistory::new(1000).map(Box::new);
    let mut ed = Reedline::create()
        .with_completer(Box::new(Complete { units }))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(menu)))
        .with_hinter(Box::new(DefaultHinter::default()))
        .with_edit_mode(Box::new(Emacs::new(keys)))
        .with_quick_completions(true)
        .with_partial_completions(true);
    if let Ok(h) = history {
        ed = ed.with_history(h);
    }
    ed
}

pub fn prompt() -> DefaultPrompt {
    DefaultPrompt::new(
        DefaultPromptSegment::Basic("neoks10".into()),
        DefaultPromptSegment::Empty,
    )
}

/// Ask the engine and wait for the reply.
pub fn ask(tx: &Sender<Request>, cmd: Command) -> Reply {
    let (rtx, rrx) = channel();
    if tx
        .send(Request::Command {
            cmd,
            from: 0,
            reply: rtx,
        })
        .is_err()
    {
        return Reply::err("the machine has stopped");
    }
    rrx.recv()
        .map(|r| r.0)
        .unwrap_or_else(|_| Reply::err("no reply"))
}

fn units(tx: &Sender<Request>) -> Units {
    let (utx, urx) = channel();
    let _ = tx.send(Request::Units(utx));
    urx.recv().unwrap_or_default()
}

/// Print the CTY output and the notices; hold them while `held` is set.
fn printer(rx: Receiver<Out>, held: Arc<AtomicBool>) {
    let mut wait: Vec<Out> = Vec::new();
    let show = |o: &Out| {
        let mut so = std::io::stdout().lock();
        match o {
            Out::Cty(b) => {
                let b: Vec<u8> = b.iter().map(|c| c & 0o177).filter(|&c| c != 0).collect();
                let _ = so.write_all(&b);
            }
            Out::Notice(t) => {
                let _ = write!(so, "\r\n{t}\r\n");
            }
            Out::Flush => {}
        }
        let _ = so.flush();
    };
    for o in rx {
        if held.load(Ordering::SeqCst) {
            if o != Out::Flush {
                wait.push(o);
            }
            continue;
        }
        for w in wait.drain(..) {
            show(&w);
        }
        show(&o);
    }
}

/// The terminal loop. Returns when the operator quits or the machine
/// thread ends.
/// The bytes that a key sends on a terminal, for the CTY. `None` for
/// the escape characters and for keys without bytes.
pub fn key_bytes(k: &KeyEvent) -> Option<Vec<u8>> {
    terminal_bytes(k).filter(|b| !is_escape_bytes(b))
}

fn is_escape_bytes(b: &[u8]) -> bool {
    b == [ESCAPE] || b == [CONSOLE_ESCAPE]
}

/// The bytes that a key sends on a terminal.
fn terminal_bytes(k: &KeyEvent) -> Option<Vec<u8>> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let mut b = match k.code {
        KeyCode::Char(c) if ctrl => {
            // crossterm reports bytes 034 to 037 as Ctrl-4 to Ctrl-7.
            let v = match c {
                '4'..='7' => 0o34 + (c as u8 - b'4'),
                ' ' | '2' | '@' => 0,
                c if c.is_ascii() => (c.to_ascii_uppercase() as u8) & 0o37,
                _ => return None,
            };
            vec![v]
        }
        KeyCode::Char(c) if c.is_ascii() => vec![c as u8],
        KeyCode::Char(_) => return None,
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0o177],
        KeyCode::Esc => vec![0o33],
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        _ => return None,
    };
    if alt {
        b.insert(0, 0o33);
    }
    Some(b)
}

fn is_escape(k: &KeyEvent) -> bool {
    terminal_bytes(k).is_some_and(|b| is_escape_bytes(&b))
}

/// Text in raw mode: each line ends with CR LF.
fn say(t: &str) {
    let mut so = std::io::stdout().lock();
    for line in t.trim_end().lines() {
        let _ = write!(so, "{line}\r\n");
    }
    let _ = so.flush();
}

/// The terminal loop. Returns when the operator quits or the machine
/// thread ends. One reader: crossterm reads the keys in both modes, so
/// that no key waits in a buffer of the other mode.
pub fn run(tx: Sender<Request>, ended: &AtomicBool) {
    // Raw mode before the first CTY output, so that the terminal does not
    // change its line ends.
    if crossterm::terminal::enable_raw_mode().is_err() {
        eprintln!("neoks10: the terminal has no raw mode");
        return;
    }
    let held = Arc::new(AtomicBool::new(false));
    let (otx, orx) = channel();
    let _ = tx.send(Request::Subscribe(otx.clone()));
    let h = held.clone();
    std::thread::spawn(move || printer(orx, h));
    let shared = Arc::new(Mutex::new(units(&tx)));
    let mut ed = editor(shared.clone());
    let prompt = prompt();
    loop {
        if ended.load(Ordering::SeqCst) {
            break;
        }
        match crossterm::event::poll(std::time::Duration::from_millis(100)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => break,
        }
        let Ok(ev) = crossterm::event::read() else {
            break;
        };
        let Event::Key(k) = ev else { continue };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        if !is_escape(&k) {
            if let Some(b) = key_bytes(&k)
                && tx.send(Request::Keys(b)).is_err()
            {
                break;
            }
            continue;
        }
        held.store(true, Ordering::SeqCst);
        say("");
        let quit = command_mode(&tx, &mut ed, &prompt, &shared);
        held.store(false, Ordering::SeqCst);
        let _ = otx.send(Out::Flush);
        if quit {
            break;
        }
    }
    let _ = crossterm::terminal::disable_raw_mode();
}

/// The prompt, until it closes. True if the operator quits. The terminal
/// is in raw mode also while a command runs, so that Ctrl-C and Ctrl-\
/// (SIGQUIT) do not stop the program.
fn command_mode(
    tx: &Sender<Request>,
    ed: &mut Reedline,
    prompt: &DefaultPrompt,
    shared: &Mutex<Units>,
) -> bool {
    loop {
        let line = ed.read_line(prompt);
        let _ = crossterm::terminal::enable_raw_mode();
        let line = match line {
            Ok(Signal::Success(l)) => l,
            Ok(_) => return false,
            Err(e) => {
                say(&format!("neoks10: {e}"));
                return false;
            }
        };
        let cmd = match cmd::parse(&line) {
            Ok(None) | Ok(Some(Command::Cty)) => return false,
            Ok(Some(c)) => c,
            Err(e) => {
                say(&e);
                continue;
            }
        };
        let quit = cmd == Command::Quit;
        let r = ask(tx, cmd);
        print_reply(&r);
        if quit {
            return true;
        }
        // The units may have changed (attach, mount).
        if let Ok(mut u) = shared.lock() {
            *u = units(tx);
        }
    }
}

pub fn print_reply(r: &Reply) {
    if r.ok {
        say(&r.text);
    } else {
        say(&format!("error: {}", r.text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(c, m)
    }

    #[test]
    fn keys() {
        let n = KeyModifiers::NONE;
        let c = KeyModifiers::CONTROL;
        assert_eq!(key_bytes(&key(KeyCode::Char('a'), n)), Some(b"a".to_vec()));
        assert_eq!(key_bytes(&key(KeyCode::Char('z'), c)), Some(vec![0o32]));
        assert_eq!(key_bytes(&key(KeyCode::Char('c'), c)), Some(vec![3]));
        assert_eq!(key_bytes(&key(KeyCode::Char('6'), c)), Some(vec![0o36]));
        assert_eq!(key_bytes(&key(KeyCode::Enter, n)), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(&key(KeyCode::Backspace, n)), Some(vec![0o177]));
        assert_eq!(
            key_bytes(&key(KeyCode::Char('g'), KeyModifiers::ALT)),
            Some(b"\x1bg".to_vec())
        );
        // Ctrl-] (035) and Ctrl-\ (034) open the prompt; they are not
        // keys for the CTY. A terminal reports them as Ctrl-5 and Ctrl-4,
        // or as Ctrl-] and Ctrl-\.
        for ch in ['5', ']', '4', '\\'] {
            assert!(is_escape(&key(KeyCode::Char(ch), c)), "{ch}");
            assert_eq!(key_bytes(&key(KeyCode::Char(ch), c)), None, "{ch}");
        }
        assert!(!is_escape(&key(KeyCode::Char('6'), c)));
        assert!(!is_escape(&key(KeyCode::Char(']'), n)));
    }
}
