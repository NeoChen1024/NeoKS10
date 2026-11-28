//! Boot ITS twice from copies of the disk image of the ITS build: one
//! machine runs the timer, the 8080 and the devices before each
//! instruction (`devices_each_step`), the other one only at
//! `Machine::next_event` and after a wake. The state of the two must be
//! the same at every check, and the two disk copies must be the same at
//! the end. The image is a third-party file and is not in git: the test
//! runs only if KS10_ITS_DISK names it.

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

/// The work of tests/mips/mips.script after the boot: a login and an
/// assembly with MIDAS, which does disk I/O and CTY output.
fn script() -> Script {
    let mut s = Script::default();
    s.answer("DSKDMP", "its\r");
    s.answer("\r", "\x1bg");
    s.answer("DELRNM", "\x1a");
    s.answer("Fair", ":login neoks\r");
    s.answer("LOGIN  NEOKS", ":midas sys1;ts stink_sysen2;stink\r");
    s
}

fn state(m: &Machine) -> String {
    format!(
        "{} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {}",
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
        m.pager,
        m.io.requests(),
    )
}

#[test]
fn next_event_runs_its_as_each_step() {
    let Some(src) = std::env::var_os("KS10_ITS_DISK") else {
        eprintln!("KS10_ITS_DISK is not set: next_event test not run");
        return;
    };
    let (mut a, ca) = machine(&src, "next-event-a-rp0.dsk");
    let (mut b, cb) = machine(&src, "next-event-b-rp0.dsk");
    a.devices_each_step = true;
    let (mut sa, mut sb) = (script(), script());
    let mut done = false;
    while a.count < 400_000_000 && !done {
        sa.tick(a.count, &mut a.console);
        sb.tick(b.count, &mut b.console);
        a.step().expect("step with the devices at each step");
        b.step().expect("step with next_event");
        if a.count.is_multiple_of(1_000_000) || a.count != b.count {
            assert_eq!(state(&a), state(&b), "at {}", a.count);
            assert!(a.mem == b.mem, "memory at {}", a.count);
            done = String::from_utf8_lossy(&a.console.output).contains(":KILL");
        }
    }
    assert_eq!(state(&a), state(&b));
    assert!(a.mem == b.mem);
    assert!(done, "{}", String::from_utf8_lossy(&a.console.output));
    let end = a.count;
    drop((a, b));
    let same = std::fs::read(&ca).unwrap() == std::fs::read(&cb).unwrap();
    let _ = std::fs::remove_file(&ca);
    let _ = std::fs::remove_file(&cb);
    assert!(same, "the disk copies differ");
    eprintln!("the same state through {end} steps");
}
