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
use ks10_csl::board::{Config, Csl, Stop};
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

/// The stop: SM1 starts the KS10 clock (`CSCMD`, 3 to register 206),
/// which the model does not have yet.
const CLOCK_STOP: Stop = Stop::Ks10 {
    pc: 0o6173,
    port: 0o206,
    value: 0o3,
    what: "the KS10 clock (register 206)",
};

/// The boot from the disk of the ITS build, with an empty control RAM:
/// the 8080 reads the RAM file and loads the control RAM. The expected
/// control RAM is the one that spec/micro.py assembles from the ITS
/// microcode sources (`Machine::its`); the RAM file on the disk comes
/// from the MICRO of the ITS build.
#[test]
fn the_boot_loads_the_control_ram_from_the_disk() {
    let Some(rom) = rom() else { return };
    let Some(disk) = std::env::var_os("KS10_ITS_DISK").filter(|d| !d.is_empty()) else {
        eprintln!("SKIPPED: KS10_ITS_DISK is not set");
        return;
    };
    let empty = vec![Microword::default(); CRAM_WORDS];
    let mut m = Machine::new(empty, ucode::DROM.to_vec(), W40::ZERO);
    let f = std::fs::File::open(disk).expect("the disk image");
    m.io.disk.attach(0, f, true);
    let mut c = board(rom, m);
    match c.run(200_000_000) {
        Err(s) if s == CLOCK_STOP => {}
        r => panic!("{:?}, not {CLOCK_STOP}", r.map_err(|s| s.to_string())),
    }
    assert_eq!(c.cty_output(), b"\r\nKS10 CSL.V5.2\r\nBT AUTO\r\n");
    let f: Vec<Function> = c.interface().cycles.iter().map(|b| b.function).collect();
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
    assert_eq!(after[3 * 2048..], [0o1000, 0o1103, 0o1004]);
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
    // SM1: all ones in location 0 (DMINT), the CRAM address 0 (CADWR) with
    // the microinstruction register clear.
    assert_eq!(k.memory[0], W36::ONES);
    assert_eq!((k.diag_address, k.mir_clear), (0, true));
    eprintln!("the KS10 clock starts after {} states", c.now());
}
