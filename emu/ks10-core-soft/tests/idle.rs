//! Boot ITS twice from copies of the disk image of the ITS build: one
//! machine does steps only, the other one does `Machine::step_idle`. The
//! state of the two must be the same at every count where the second
//! one stops, and the two disk copies must be the same at the end. The
//! image is a third-party file and is not in git: the test runs only if
//! KS10_ITS_DISK names it.

use ks10_core_soft::Machine;
use ks10_machine::boot;
use ks10_machine::console::Script;

fn machine(src: &std::ffi::OsStr, name: &str) -> (Machine, std::path::PathBuf) {
    let copy = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::copy(src, &copy).expect("copy the disk image");
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&copy)
        .expect("open the copy");
    let mut m = Machine::new();
    m.io.disk.attach(0, f, false);
    boot::boot_disk(&mut m, 0).expect("BT");
    (m, copy)
}

fn script() -> Script {
    let mut s = Script::default();
    s.answer("DSKDMP", "its\r");
    s.answer("\r", "\x1bg");
    s.answer("DELRNM", "\x1a");
    s.answer("Fair", ":login neoks\r");
    s.answer("LOGIN  NEOKS", ":listf sys;\r");
    s
}

fn state(m: &Machine) -> String {
    format!(
        "{} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
        m.count,
        m.pc,
        m.flags,
        m.ac,
        m.blocks,
        m.apr,
        m.pi,
        m.timer,
        m.io.events,
        m.console,
        m.pager
    )
}

#[test]
fn idle_skip_boots_its_the_same() {
    let Some(src) = std::env::var_os("KS10_ITS_DISK") else {
        eprintln!("KS10_ITS_DISK is not set: ITS idle test not run");
        return;
    };
    let (mut a, ca) = machine(&src, "idle-a-rp0.dsk");
    let (mut b, cb) = machine(&src, "idle-b-rp0.dsk");
    let (mut sa, mut sb) = (script(), script());
    let mut skipped = 0u64;
    let mut check = 0u64;
    let mut seen = false;
    while b.count < 800_000_000 && !seen {
        sb.tick(b.count, &mut b.console);
        let limit = sb.due().unwrap_or(u64::MAX).min(b.count + 100_000);
        skipped += b.step_idle(limit).expect("step_idle");
        while a.count < b.count {
            sa.tick(a.count, &mut a.console);
            a.step().expect("step");
        }
        if b.count >= check {
            check = b.count + 1_000_000;
            assert_eq!(state(&a), state(&b), "at {}", b.count);
            assert!(a.mem == b.mem, "memory at {}", b.count);
            seen = String::from_utf8_lossy(&b.console.output).contains("FREE BLOCKS");
        }
    }
    assert_eq!(state(&a), state(&b));
    assert!(a.mem == b.mem);
    assert!(seen, "{}", String::from_utf8_lossy(&b.console.output));
    let end = b.count;
    drop((a, b));
    let same = std::fs::read(&ca).unwrap() == std::fs::read(&cb).unwrap();
    let _ = std::fs::remove_file(&ca);
    let _ = std::fs::remove_file(&cb);
    assert!(same, "the disk copies differ");
    eprintln!("{skipped} of {end} steps skipped");
}
