//! The power-up of the console program version 5.2 on the model of the
//! board with the microcode-level model as the KS10, up to the boot.
//!
//! The expected values come from the listing KNS10.PNT: `EM1` reads
//! memory location 0 (`MEMAD` is 0 after `CLRLP`), `EBCMD` examines the
//! bus, `SYSUP` waits and prints `BT AUTO` (`AUTOMS`), and `BTAUT` runs
//! the channel command list `DSKSEQ` (`CHNXCT`).

use ks10_core_uop::alu::W40;
use ks10_core_uop::cram::{CRAM_WORDS, Microword};
use ks10_core_uop::machine::Machine;
use ks10_core_uop::ucode;
use ks10_csl::board::{Config, Csl};
use ks10_csl::ks10::{Function, Reply};
use ks10_csl::rom;
use pdp10::W36;

fn rom() -> Option<Vec<u8>> {
    let Some(d) = rom::dir() else {
        eprintln!("SKIPPED: KS10_CSL_DIR is not set (make csl)");
        return None;
    };
    Some(rom::load(&rom::image_dir(&d)).expect("the ROM images"))
}

fn board(rom: Vec<u8>, m: Machine) -> Csl<Machine> {
    let config = Config {
        switches: 0o140,
        cty_carrier: false,
        klinik_carrier: false,
        char_states: 0,
    };
    Csl::new(rom, config, m).unwrap()
}

/// The command/address word of an I/O cycle (EK-OKS10-TM-002 5.3.4: bit 0
/// I/O, bit 1 read, bit 2 write) on UBA 1 (`DSKUBA` = 4 in `PRMLST`).
fn io(write: bool, register: u64) -> W36 {
    let function = if write { 0o500001 } else { 0o600001 };
    W36::wrap(function << 18 | register)
}

/// The registers of the RH11 at 776700 (`.DSBASE`).
const RPCS1: u64 = 0o776700;
const RPWC: u64 = 0o776702;
const RPBA: u64 = 0o776704;
const RPDA: u64 = 0o776706;
const RPCS2: u64 = 0o776710;
const RPDS: u64 = 0o776712;
const RPDC: u64 = 0o776734;

/// The bus cycles of `DSKSEQ` from its start to `TWAIT 400`: the UBA page
/// map, the unit number, and the drive status twice (`EI.`, then `EI1`
/// in `CMDTWAIT`).
fn dskseq_start(f: &[Function]) {
    let w = |register, data| Function::IoWrite {
        com_adr: io(true, register),
        data: W36::wrap(data),
        taken: true,
    };
    // UBA. 763,001 and DI. 140,001: page 1 VALID and 36-BIT XFR.
    assert_eq!(f[0], w(0o763001, 0o140001));
    // LI. D776,P.10 and DI.INDIRECT UNITNM: unit 0.
    assert_eq!(f[1], w(RPCS2, 0));
    for k in [2, 3] {
        assert!(
            matches!(f[k], Function::IoRead { com_adr, .. } if com_adr == io(false, RPDS)),
            "{:?}",
            f[k]
        );
    }
}

#[test]
fn without_a_disk_the_boot_fails_at_the_drive_check() {
    let Some(rom) = rom() else { return };
    let word = W36::wrap(0o123456_701234);
    let mut m = Machine::its(W40::ZERO);
    m.memory[0] = word;
    let mut c = board(rom, m);
    // The null job waits for input after the prompt.
    assert_eq!(c.run(100_000_000).map_err(|s| s.to_string()), Ok(()));
    // A.BTERR adds 2 to the high byte; DEVERR puts the address of the
    // failed command + 2 in the low byte: TWAIT 400 at 017060, so 062.
    // REINI prints the prompt, then the byte 377 of KSPRMT, which the
    // 7-bit CTY sends as 177.
    assert_eq!(
        String::from_utf8_lossy(c.cty_output()),
        "\r\nKS10 CSL.V5.2\r\nBT AUTO\r\n?BT 001062\r\n\r\nKS10>\u{7f}"
    );
    let f: Vec<Function> = c.interface().cycles.iter().map(|b| b.function).collect();
    assert_eq!(f.len(), 7, "{f:#?}");
    // EM1: a read (bit 1) of location 0. EBCMD: no device drives the bus.
    let read0 = Function::MemoryRead {
        com_adr: W36::wrap(0o200000_000000),
        reply: Reply::Data(word),
    };
    assert_eq!(f[..2], [read0, Function::ExamineBus { data: W36::ZERO }]);
    dskseq_start(&f[2..6]);
    // No drive: DPR (RPDS bit 8, the bit of TWAIT 400) is clear.
    for k in [4, 5] {
        let Function::IoRead {
            reply: Reply::Data(ds),
            ..
        } = f[k]
        else {
            panic!("{:?}", f[k]);
        };
        assert!(!ds.bit(35 - 8), "{ds:?}");
    }
    // The null job reads the REMOTE DIAG switches as PROTECT (no panel,
    // the pull-ups); without a password, KLNKLT and SETM0 hang up the line:
    // HANGUP gives CARRLOSS (3) to WRD34, which deposits it in bits 20 to 27
    // of word 34 (a write, bit 2) and sets INT 10.
    assert_eq!(
        f[6],
        Function::MemoryWrite {
            com_adr: W36::wrap(0o100000_000034),
            data: W36::wrap(3 << 8),
            taken: true,
        }
    );
    assert_eq!(c.ks10().memory[0o34], W36::wrap(3 << 8));
    // The 8080 program serves the CTY, not the service of the model.
    assert!(c.ks10().csl_model);
    assert!(c.interface().int_10);
}

/// A copy of the disk image in the build output: the boot writes to the
/// disk.
fn disk_copy(name: &str) -> Option<std::fs::File> {
    let Some(src) = std::env::var_os("KS10_ITS_DISK").filter(|d| !d.is_empty()) else {
        eprintln!("SKIPPED: KS10_ITS_DISK is not set");
        return None;
    };
    let copy = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::copy(src, &copy).expect("copy the disk image");
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&copy)
        .expect("open the copy");
    Some(f)
}

/// The prompt of the disk program, as the 8080 prints it after BT.
const PROMPT: &str = "DSKDMP";

/// The CTY output of the boot on `ks10-core-uop` with the high-level
/// console of `ks10-machine` (`boot::boot_disk`), up to the end of the
/// prompt: the expected text from another implementation of the console.
fn high_level_boot(f: std::fs::File) -> String {
    let mut m = Machine::its(W40::ZERO);
    m.io.disk.attach(0, f, false);
    ks10_machine::boot::boot_disk(&mut m, 0).expect("BT on the model");
    for _ in 0..200_000_000u64 {
        m.step().expect("a run of the model");
        let out = String::from_utf8_lossy(&m.cty.output).into_owned();
        if let Some(k) = out.find(PROMPT) {
            return out[..k + PROMPT.len()].to_string();
        }
    }
    panic!("no prompt from the high-level console");
}

/// The boot from the disk of the ITS build, with an empty control RAM:
/// the 8080 reads the RAM file, loads the control RAM, starts the
/// microcode (SM1) and the pre-boot (STINT), and serves the CTY output of
/// the disk program. The expected control RAM is the one that
/// spec/micro.py assembles from the ITS microcode sources
/// (`Machine::its`); the RAM file on the disk comes from the MICRO of the
/// ITS build.
#[test]
fn the_boot_loads_the_control_ram_and_starts_the_disk_program() {
    let Some(rom) = rom() else { return };
    let Some(f) = disk_copy("csl-boot.dsk") else {
        return;
    };
    let empty = vec![Microword::default(); CRAM_WORDS];
    let mut m = Machine::new(empty, ucode::DROM.to_vec(), W40::ZERO);
    m.io.disk.attach(0, f, false);
    let mut c = board(rom, m);
    let banner = "\r\nKS10 CSL.V5.2\r\nBT AUTO\r\n";
    let mut out = String::new();
    while c.now() < 200_000_000 && !out.contains(PROMPT) {
        if let Err(s) = c.run(1_000_000) {
            panic!("{s}; processor {:?}; CTY {out:?}", c.ks10().pending_stop);
        }
        out = String::from_utf8_lossy(c.cty_output()).into_owned();
    }
    eprintln!("the prompt after {} states", c.now());
    // The first start of the KS10 clock: CSCMD of SM1 (206/3).
    let start = c
        .writes()
        .iter()
        .find(|w| (w.port, w.value) == (0o206, 0o3))
        .expect("SM1")
        .at;
    let f: Vec<Function> = c
        .interface()
        .cycles
        .iter()
        .filter(|b| b.at < start)
        .map(|b| b.function)
        .collect();
    let first_latch = f
        .iter()
        .position(|x| matches!(x, Function::CraLatch { .. }))
        .expect("a CRAM load");
    // Before the load: each DSKSEQ writes RPDA, then RPDC; the memory reads
    // between them are the checks of the program.
    let mut disk = Vec::new();
    let mut reads = Vec::new();
    for x in &f[2..first_latch] {
        match *x {
            Function::IoWrite { com_adr, data, .. } if com_adr == io(true, RPDA) => {
                disk.push((data, W36::ZERO));
            }
            Function::IoWrite { com_adr, data, .. } if com_adr == io(true, RPDC) => {
                disk.last_mut().unwrap().1 = data;
            }
            Function::MemoryRead {
                com_adr,
                reply: Reply::Data(w),
            } => {
                reads.push((com_adr.field(16, 35), w));
            }
            _ => {}
        }
    }
    let addresses: Vec<u64> = reads.iter().map(|r| r.0).collect();
    // FILESH: the home block is block 1 of cylinder 0; CHKHOM finds HOMEWD
    // ("HOM" in SIXBIT) at 1000. GOODPK: the pointer page from word 1103.
    // MICROP: the first page of the microcode from word 1002.
    assert_eq!(addresses, [0o1000, 0o1103, 0o1002], "{reads:?}");
    assert_eq!(reads[0].1, W36::wrap(0o505755_000000));
    // BLKRDR: the cylinder from bits 0 to 11 (EMBUF+3), the track and
    // sector from bits 20 to 35 (EMBUF).
    let split = |w: W36| (W36::wrap(w.field(20, 35)), W36::wrap(w.field(0, 11)));
    assert_eq!(
        disk,
        [
            (W36::wrap(1), W36::ZERO),
            split(reads[1].1),
            split(reads[2].1)
        ]
    );
    // Each transfer of QXFR before the load: 512 words (176000) to address
    // 1000 (Unibus 4000), with the read command 71.
    for (register, data) in [(RPWC, 0o176000), (RPBA, 0o4000), (RPCS1, 0o71)] {
        let n = f[..first_latch]
            .iter()
            .filter(|x| {
                matches!(x, Function::IoWrite { com_adr, data: d, .. }
                    if *com_adr == io(true, register) && *d == W36::wrap(data))
            })
            .count();
        assert_eq!(n, 3, "register {register:o}");
    }
    // MEM2CRAM: CADWR at 0, eight WFUNC and one CADWR for each of the
    // 2048 words; then SM1 writes the CRAM address 0 (CADWR).
    let latches = f
        .iter()
        .filter(|x| matches!(x, Function::CraLatch { .. }))
        .count();
    assert_eq!(latches, 1 + 2048 * 9 + 1);
    // Three memory words for each control RAM word (RD.EXM), then LBINT
    // reads the pointer to the monitor pre-boot through FILEINIT: the home
    // block at 1000, the pointer page from 1103, and word 1004 (1000 plus
    // RM100, 4 for the monitor, LOAD4).
    let after: Vec<u64> = f[first_latch..]
        .iter()
        .filter_map(|x| match *x {
            Function::MemoryRead { com_adr, .. } => Some(com_adr.field(16, 35)),
            _ => None,
        })
        .collect();
    assert_eq!(after.len(), 3 * 2048 + 3);
    let pages = (0..12u64).flat_map(|_| (0..512u64).map(|k| 0o1000 + k));
    assert!(after[..3 * 2048].iter().copied().eq(pages));
    assert_eq!(after[3 * 2048..], [0o1000, 0o1103, 0o1002 + 2]);
    let k = c.ks10();
    let its = Machine::its(W40::ZERO);
    let wrong: Vec<usize> = (0..CRAM_WORDS)
        .filter(|&a| k.cram[a] != its.cram[a])
        .collect();
    assert!(
        wrong.is_empty(),
        "{} words differ, first at {:o}",
        wrong.len(),
        wrong[0]
    );
    // PASSSRC deposits the RH11 base with the UBA number (.DSBASE 776700,
    // DSKUBA 4 in bits 12 to 19) in 36, the unit in 37 and DEN.SLV
    // (D 0,0,,2,0) in 40 (PRMLST). The disk program uses these locations
    // later, so the test reads the bus log, not the memory.
    let deposits: Vec<(u64, W36)> = c
        .interface()
        .cycles
        .iter()
        .filter_map(|b| match b.function {
            Function::MemoryWrite { com_adr, data, .. } if b.at > start => {
                Some((com_adr.field(16, 35), data))
            }
            _ => None,
        })
        .filter(|(a, _)| (0o36..=0o40).contains(a))
        .collect();
    assert_eq!(
        deposits,
        [
            (0o36, W36::wrap(0o1_776700)),
            (0o37, W36::ZERO),
            (0o40, W36::wrap(0o2000)),
        ]
    );
    // After the banner, the CTY output is that of the disk program, as the
    // high-level console gives it.
    let Some(f2) = disk_copy("csl-boot-high-level.dsk") else {
        return;
    };
    let want = format!("{banner}{}", high_level_boot(f2));
    let got = &out[..out.find(PROMPT).unwrap() + PROMPT.len()];
    assert_eq!(got, want);
}

/// One character on a line of 9600 baud, in 8080 states: 10 / 9600 s at
/// 1.6384 MHz (EK-OKS10-TM-002 5.8.1). `PWRUP` sets 7 or 8 data bits and
/// 1 or 2 stop bits from the switches; 10 bits for each character and the
/// baud rate (docs/unknowns.md) are choices of the test.
const CHAR_STATES: u64 = 1707;

/// The time between two typed characters: 100 ms, the speed of a person.
const TYPING_STATES: u64 = 163_840;

/// The CTY input word of the 8080 program (`CTYIN`).
const CTYIN: usize = 0o32;

/// The CTY output of the boot on `ks10-core-uop` with the high-level
/// console of `ks10-machine`, typed on like `ks10-core-uop/tests/its_boot.rs`
/// does, up to `done`.
fn high_level_its(f: std::fs::File, done: &str) -> String {
    let mut m = Machine::its(W40::ZERO);
    m.io.disk.attach(0, f, false);
    ks10_machine::boot::boot_disk(&mut m, 0).expect("BT on the model");
    let mut script = ks10_machine::console::Script::default();
    script.answer("DSKDMP", "its\r");
    script.answer("\r", "\x1bg");
    let mut last = u64::MAX;
    while m.instructions < 100_000_000 {
        if m.instructions != last {
            last = m.instructions;
            script.tick(m.now(), &mut m.cty);
            let out = String::from_utf8_lossy(&m.cty.output);
            if let Some(k) = out.find(done) {
                return out[..k + done.len()].to_string();
            }
        }
        m.step().expect("a run of the model");
    }
    panic!("no {done:?} from the high-level console");
}

/// The 8080 program serves the CTY input of the disk program and of ITS:
/// `its` at the prompt of the disk program, then ESC G, as
/// `ks10-core-uop/tests/its_boot.rs` types. The program deposits each
/// character in word 32 and does not wait for the KS10 to take the one
/// before (hcore.m80, `TENCHR`); the disk program does not read the CTY
/// while it loads ITS. So the test types a character only after the KS10
/// has cleared word 32.
///
/// The CTY output up to `IN OPERATION` is the output of the high-level
/// console after the banner, except for the place of the bell. ITS sends
/// ^G without a cursor motion (ts3tty, `TYOBEL`, `TYONCC`), so a pending
/// space can come before or after it: with 1707 and 3000 states for each
/// character the space comes first, with 17067 states the bell comes
/// first, as on the high-level console (2026-11-28, this test).
#[test]
fn the_8080_program_passes_the_cty_input_to_its() {
    const DONE: &str = "IN OPERATION";
    let Some(rom) = rom() else { return };
    let Some(f) = disk_copy("csl-its.dsk") else {
        return;
    };
    let empty = vec![Microword::default(); CRAM_WORDS];
    let mut m = Machine::new(empty, ucode::DROM.to_vec(), W40::ZERO);
    m.io.disk.attach(0, f, false);
    let config = Config {
        switches: 0o140,
        cty_carrier: false,
        klinik_carrier: false,
        char_states: CHAR_STATES,
    };
    let mut c = Csl::new(rom, config, m).unwrap();
    let mut answers = vec![("DSKDMP", "its\r"), ("\r", "\x1bg")].into_iter();
    let mut next = answers.next();
    let mut typing = std::collections::VecDeque::new();
    let (mut from, mut typed) = (0, 0);
    let mut out = String::new();
    while c.now() < 400_000_000 && !out.contains(DONE) {
        if let Err(s) = c.run(10_000) {
            panic!("{s}; processor {:?}; CTY {out:?}", c.ks10().pending_stop);
        }
        if c.cty_output().len() != out.len() {
            out = String::from_utf8_lossy(c.cty_output()).into_owned();
        }
        if typing.is_empty()
            && let Some((prompt, text)) = next
            && out[from..].contains(prompt)
        {
            typing.extend(text.bytes());
            next = answers.next();
        }
        if c.cty_input_pending() == 0
            && c.ks10().memory[CTYIN] == W36::ZERO
            && c.now() >= typed + TYPING_STATES
            && let Some(b) = typing.pop_front()
        {
            c.type_cty(&[b]);
            typed = c.now();
            if typing.is_empty() {
                from = out.len();
            }
        }
    }
    eprintln!(
        "after {} states, {} instructions",
        c.now(),
        c.ks10().instructions
    );
    let got = &out[..out.find(DONE).unwrap_or_else(|| panic!("{out:?}")) + DONE.len()];
    let Some(f2) = disk_copy("csl-its-high-level.dsk") else {
        return;
    };
    let want = format!(
        "\r\nKS10 CSL.V5.2\r\nBT AUTO\r\n{}",
        high_level_its(f2, DONE)
    );
    let bells = |t: &str| t.matches('\x07').count();
    assert_eq!(bells(got), bells(&want));
    assert_eq!(got.replace('\x07', ""), want.replace('\x07', ""));
}
