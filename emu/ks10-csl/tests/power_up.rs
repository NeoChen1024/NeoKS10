//! The power-up of the console program version 5.2 on the model of the
//! board without a KS10, up to its first bus function.
//!
//! The expected values come from the listing KNS10.PNT (labels `PWRUP`,
//! `MRINT`, `SMFINI`, `STATEM`, `XXX230`, `XXX235`, `INIMS`, `EM1`,
//! `ADATP`, `BUSRESP`, `CHKERR`), not from a run of the model. The ROM
//! checksum of `PWRUP` is a test of the 8080 also: an error in an
//! instruction that it uses prints `?CHK`.

use ks10_csl::board::{Config, Csl, Stop, Write};
use ks10_csl::ks10::Detached;
use ks10_csl::rom;

/// The switches of port 300 (`TTYSW`), bits 7 to 4 as the 8080 reads
/// them. `PWRUP` complements the port and takes the CTY stop bit from bit
/// 7, the CTY length bit from bit 6, the KLINIK stop bit from bit 5 and
/// the KLINIK length bit from bit 4. This value gives 1001 after the
/// complement: two stop bits and 7 data bits on the CTY (mode 312), one
/// stop bit and 8 data bits on the KLINIK line (mode 116).
const SWITCHES: u8 = 0o140;

/// The banner `INIMS`: a backslash is a CR and an LF (`PLNE`).
const BANNER: &[u8] = b"\r\nKS10 CSL.V5.2\r\n";

/// The writes from power-up to the bus function of `EMCONT`, without the
/// characters of the USARTs. `PARBT`, `TRAPEN` and `STATE` are zero at
/// the first `MRINT`: `CLRLP` cleared the RAM, and `PRMLST` comes later.
const WRITES: [(u8, u8); 22] = [
    (0o212, 0o000), // PWRUP: OUT CPUCTL
    (0o204, 0o005), // MRINT: OUT CRMCTL
    (0o100, 0o200), // MRINT: OUT RESET
    (0o100, 0o000), // SMFINI: PARBT
    (0o205, 0o000), // MRINT: TRAPEN to DIAG
    (0o101, 0o000), // STATEM: STATE & 012 to LIGHTS
    (0o201, 0o312), // CTY mode
    (0o203, 0o116), // KLINIK mode
    (0o201, 0o025), // CTY command: TxEN, RxE, ER
    (0o203, 0o020), // KLINIK command: ER
    (0o100, 0o174), // XXX230: DEFLTE to SMPAR
    (0o203, 0o025), // XXX235: KLINIK command
    (0o101, 0o010), // XXX235: DTR
    (0o210, 0o000), // 5.2C: clear NXM
    (0o103, 0o000), // ADATP: MEMAD, bits 28-35
    (0o105, 0o000),
    (0o107, 0o000),
    (0o111, 0o000),
    (0o113, 0o000),
    (0o113, 0o004), // EM.CRM: read function
    (0o115, 0o004), // COM/ADR cycle
    (0o210, 0o343), // EMCONT
];

/// The stop: `EMCONT` starts a memory read (343 to register 210), and the
/// board has no KS10.
const STOP: Stop = Stop::Ks10 {
    pc: 0o5017,
    port: 0o210,
    value: 0o343,
    what: "a bus function without a KS10",
};

/// More 8080 states than the power-up needs, at the longest character time
/// of the tests.
const LIMIT: u64 = 20_000_000;

fn rom() -> Option<Vec<u8>> {
    let Some(d) = rom::dir() else {
        eprintln!("SKIPPED: KS10_CSL_DIR is not set (make csl)");
        return None;
    };
    Some(rom::load(&rom::image_dir(&d)).expect("the ROM images"))
}

fn power_up(rom: Vec<u8>, char_states: u64) -> Csl<Detached> {
    let config = Config {
        switches: SWITCHES,
        cty_carrier: false,
        klinik_carrier: false,
        char_states,
    };
    let mut c = Csl::new(rom, config, Detached::default()).unwrap();
    let cty = |c: &Csl<Detached>| String::from_utf8_lossy(c.cty_output()).into_owned();
    match c.run(LIMIT) {
        Err(s) if s == STOP => {}
        Err(s) => panic!("{s}, not {STOP}; CTY: {:?}", cty(&c)),
        Ok(()) => panic!("no stop after {LIMIT} states; CTY: {:?}", cty(&c)),
    }
    c
}

fn writes(c: &Csl<Detached>) -> Vec<(u8, u8)> {
    c.writes()
        .iter()
        .map(|&Write { port, value, .. }| (port, value))
        .collect()
}

/// A character time for the second run: about one character of 11 bits
/// at 9600 baud with an 8080 state of 610 ns (docs/console-interface.md).
/// An example for the test: the baud rate of the board is open
/// (docs/unknowns.md).
const SLOW: u64 = 1877;

#[test]
fn the_rom_prints_its_banner() {
    let Some(rom) = rom() else { return };
    let mut now = Vec::new();
    for char_states in [0, SLOW] {
        let c = power_up(rom.clone(), char_states);
        // The run stops before the last characters leave the line.
        let mut cty = c.cty_output().to_vec();
        cty.extend(c.cty_queued());
        assert_eq!(cty, BANNER, "character time {char_states}");
        assert_eq!(c.klinik_output(), b"");
        assert_eq!(writes(&c), WRITES);
        eprintln!(
            "character time {char_states}: the banner and the stop at the bus function after {} states",
            c.now()
        );
        now.push(c.now());
    }
    // PCHR1 waits for TxRDY: with a character time, the start is longer
    // by more than one character.
    assert!(now[1] > now[0] + SLOW, "{now:?}");
}

/// One byte of `PECMD` (only the command PE runs it) in the second chip,
/// E73. `CHKERR` prints the chip number, 1 to 4, and the start continues.
#[test]
fn a_bad_rom_byte_prints_a_checksum_error() {
    let Some(mut rom) = rom() else { return };
    rom[0o7341] ^= 1;
    let c = power_up(rom, 0);
    let mut want = b"?CHK2\r\n".to_vec();
    want.extend_from_slice(BANNER);
    assert_eq!(c.cty_output(), want);
}
