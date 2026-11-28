//! The power-up of the console program version 5.2 on the model of the
//! board with the microcode-level model as the KS10.
//!
//! The expected values come from the listing KNS10.PNT: `EM1` reads
//! memory location 0 (`MEMAD` is 0 after `CLRLP`), `EBCMD` examines the
//! bus, `SYSUP` waits and prints `BT AUTO` (`AUTOMS`), and `BTAUT` starts
//! the boot.

use ks10_core_uop::alu::W40;
use ks10_core_uop::machine::Machine;
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

/// The stop: the first bus function of the boot is the write of the UBA
/// page map, `DI.` of `DSKSEQ` through `DMCONT` (160 to register 210:
/// BUS REQ, XMIT ADR, XMIT DATA, an I/O register write in EK-OKS10-TM-002
/// Table 5-10). The model has no bus write yet.
const STOP: Stop = Stop::Ks10 {
    pc: 0o5221,
    port: 0o210,
    value: 0o160,
    what: "a bus write (register 210)",
};

#[test]
fn power_up_reads_memory_and_starts_the_boot() {
    let Some(rom) = rom() else { return };
    let config = Config {
        switches: 0o140,
        cty_carrier: false,
        klinik_carrier: false,
        char_states: 0,
    };
    let word = W36::wrap(0o123456_701234);
    let mut m = Machine::its(W40::ZERO);
    m.memory[0] = word;
    let mut c = Csl::new(rom, config, m).unwrap();
    match c.run(100_000_000) {
        Err(s) if s == STOP => {}
        r => panic!(
            "{:?}, not {STOP}; CTY {:?}",
            r.map_err(|s| s.to_string()),
            String::from_utf8_lossy(c.cty_output())
        ),
    }
    assert_eq!(c.cty_output(), b"\r\nKS10 CSL.V5.2\r\nBT AUTO\r\n");
    let functions: Vec<Function> = c.interface().cycles.iter().map(|b| b.function).collect();
    assert_eq!(
        functions,
        [
            // EM1: a read (bit 1) of location 0.
            Function::MemoryRead {
                com_adr: W36::wrap(0o200000_000000),
                reply: Reply::Data(word),
            },
            // EBCMD: no device drives the bus.
            Function::ExamineBus { data: W36::ZERO },
        ]
    );
    // UBA. 763,001 and DI. 140,001 of DSKSEQ: an I/O write (bits 0 and 2)
    // to register 763001 of controller 1 (DSKUBA = 4 in PRMLST), with
    // COM/ADR CYCLE; the data VALID, 36-BIT XFR and page 1, with DATA CYCLE.
    let i = c.interface();
    assert_eq!(i.com_adr, (W36::wrap(0o500001_763001), 0o4));
    assert_eq!(i.data, (W36::wrap(0o140001), 0o1));
    eprintln!("the boot starts after {} states", c.now());
}
