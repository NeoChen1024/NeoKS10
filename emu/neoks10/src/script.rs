//! Script files (docs/emulator.md, "Scripts"): one console command for
//! each line, with `expect TEXT [--steps N]` and `send TEXT`. A line that
//! starts with `#` is a comment.
//!
//! The machine thread runs the script (`engine.rs`), so that the
//! instruction count of each step depends only on the guest: an `expect`
//! matches at the instruction that prints the text, and a `send` comes
//! when the CTY output is quiet (`ks10_machine::console::QUIET`
//! instructions without a character), after the previous step of the
//! script.

use std::collections::VecDeque;
use std::path::Path;

use crate::cmd::{self, Command};

pub struct Script {
    pub name: String,
    /// The steps with their line numbers.
    pub steps: VecDeque<(usize, Command)>,
}

impl Script {
    pub fn parse(name: &str, text: &str) -> Result<Script, String> {
        let mut steps = VecDeque::new();
        for (n, line) in text.lines().enumerate() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            match cmd::parse(t) {
                Ok(Some(Command::Cty)) => {
                    return Err(format!("{name}:{}: cty has no use in a script", n + 1));
                }
                Ok(Some(c)) => steps.push_back((n + 1, c)),
                Ok(None) => {}
                Err(e) => return Err(format!("{name}:{}: {e}", n + 1)),
            }
        }
        Ok(Script {
            name: name.to_string(),
            steps,
        })
    }

    pub fn load(path: &Path) -> Result<Script, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Script::parse(&path.display().to_string(), &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_script() {
        let s = Script::parse(
            "t",
            "# boot ITS\nboot disk\nexpect DSKDMP\nsend \"its\\r\"\n\nquit\n",
        )
        .unwrap();
        let lines: Vec<usize> = s.steps.iter().map(|x| x.0).collect();
        assert_eq!(lines, [2, 3, 4, 6]);
        assert_eq!(s.steps[2].1, Command::CtySend(b"its\r".to_vec()));
        let e = Script::parse("t", "boot disk\ntape 9 mount x\n")
            .err()
            .unwrap();
        assert!(e.starts_with("t:2: "), "{e}");
    }
}
