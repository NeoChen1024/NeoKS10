//! The C interface of the microcode-level model (`ks10-core-uop`), for the
//! co-simulation with the logic in `rtl/` (AGENTS.md, "Rules for the
//! emulator"). spec/capi.toml defines the functions, the status codes and
//! the record; `make gen` writes `tb/gen/ks10_capi.h` and [`capi`] from it.
//!
//! This crate is the second place of `unsafe` (AGENTS.md): a C caller
//! gives raw pointers. Each function checks a pointer for NULL, and no
//! panic crosses the interface: a panic gives `KS10_PANIC` and marks the
//! machine as not usable.

#![allow(unsafe_code)]

#[rustfmt::skip]
pub mod capi;

use std::ffi::{c_char, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};

pub use capi::*;
use ks10_core_uop::alu::W40;
use ks10_core_uop::machine::Machine;
use ks10_core_uop::record::Record;
use ks10_machine::boot::Processor;
use pdp10::W36;

/// A model behind the C interface (`ks10_uop`).
pub struct Ks10Uop {
    m: Machine,
    /// The reason of a stop of the model. The machine stays stopped.
    stop: Option<String>,
    /// A panic happened in a call.
    broken: bool,
}

// Each function has the type of spec/capi.toml.
const _: UopNew = ks10_uop_new;
const _: UopFree = ks10_uop_free;
const _: UopMasterReset = ks10_uop_master_reset;
const _: UopDeposit = ks10_uop_deposit;
const _: UopExamine = ks10_uop_examine;
const _: UopStart = ks10_uop_start;
const _: UopNextRecord = ks10_uop_next_record;
const _: UopStopText = ks10_uop_stop_text;

impl From<Record> for Ks10Record {
    fn from(r: Record) -> Self {
        Ks10Record {
            count: r.count,
            cycles: r.cycles,
            pc: r.pc.get(),
            flags: r.flags.get(),
            ac: r.ac.map(W36::get),
            writes: r.writes,
            write_hash: r.write_hash,
        }
    }
}

/// Run `f` on the machine of `m`, without a panic across the interface.
///
/// # Safety
/// As for the functions of the interface.
unsafe fn with_mut(m: *mut Ks10Uop, f: impl FnOnce(&mut Ks10Uop) -> c_int) -> c_int {
    // SAFETY: `m` is NULL or a pointer from `ks10_uop_new` that the caller
    // has not freed and does not use in another thread at the same time.
    let Some(u) = (unsafe { m.as_mut() }) else {
        return KS10_NULL;
    };
    if u.broken {
        return KS10_PANIC;
    }
    match catch_unwind(AssertUnwindSafe(|| f(u))) {
        Ok(s) => s,
        Err(_) => {
            u.broken = true;
            KS10_PANIC
        }
    }
}

/// Take a stop that `master_reset` or `start_at` left for the next step.
fn pending_stop(u: &mut Ks10Uop) -> c_int {
    match u.m.pending_stop.take() {
        Some(s) => {
            u.stop = Some(s.to_string());
            KS10_STOP
        }
        None => KS10_OK,
    }
}

/// `ks10_uop_new` (spec/capi.toml).
#[unsafe(no_mangle)]
pub extern "C" fn ks10_uop_new(fill: u64) -> *mut Ks10Uop {
    catch_unwind(|| {
        Box::into_raw(Box::new(Ks10Uop {
            m: Machine::its(W40::wrap(fill)),
            stop: None,
            broken: false,
        }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// `ks10_uop_free` (spec/capi.toml).
///
/// # Safety
/// `m` is NULL or a pointer from `ks10_uop_new` that is not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ks10_uop_free(m: *mut Ks10Uop) {
    if !m.is_null() {
        // SAFETY: the contract of this function.
        drop(unsafe { Box::from_raw(m) });
    }
}

/// `ks10_uop_master_reset` (spec/capi.toml).
///
/// # Safety
/// Each pointer is NULL or valid: `m` from `ks10_uop_new` and not freed,
/// and not used by another thread at the same time; an output pointer to
/// memory of the caller of the size of its type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ks10_uop_master_reset(m: *mut Ks10Uop) -> c_int {
    // SAFETY: the contract of this function.
    unsafe {
        with_mut(m, |u| {
            u.m.master_reset();
            u.stop = None;
            pending_stop(u)
        })
    }
}

/// `ks10_uop_deposit` (spec/capi.toml).
///
/// # Safety
/// Each pointer is NULL or valid: `m` from `ks10_uop_new` and not freed,
/// and not used by another thread at the same time; an output pointer to
/// memory of the caller of the size of its type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ks10_uop_deposit(m: *mut Ks10Uop, address: u32, word: u64) -> c_int {
    // SAFETY: the contract of this function.
    unsafe {
        with_mut(m, |u| {
            let (Some(w), Some(cell)) = (W36::new(word), u.m.memory.get_mut(address as usize))
            else {
                return KS10_BAD_ARG;
            };
            *cell = w;
            KS10_OK
        })
    }
}

/// `ks10_uop_examine` (spec/capi.toml).
///
/// # Safety
/// Each pointer is NULL or valid: `m` from `ks10_uop_new` and not freed,
/// and not used by another thread at the same time; an output pointer to
/// memory of the caller of the size of its type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ks10_uop_examine(
    m: *const Ks10Uop,
    address: u32,
    word: *mut u64,
) -> c_int {
    // SAFETY: as in `with_mut`, for reading.
    let Some(u) = (unsafe { m.as_ref() }) else {
        return KS10_NULL;
    };
    if word.is_null() {
        return KS10_NULL;
    }
    if u.broken {
        return KS10_PANIC;
    }
    let Some(w) = u.m.memory.get(address as usize) else {
        return KS10_BAD_ARG;
    };
    // SAFETY: `word` is not NULL and points to a uint64_t of the caller.
    unsafe { word.write(w.get()) };
    KS10_OK
}

/// `ks10_uop_start` (spec/capi.toml).
///
/// # Safety
/// Each pointer is NULL or valid: `m` from `ks10_uop_new` and not freed,
/// and not used by another thread at the same time; an output pointer to
/// memory of the caller of the size of its type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ks10_uop_start(m: *mut Ks10Uop, address: u32) -> c_int {
    // SAFETY: the contract of this function.
    unsafe {
        with_mut(m, |u| {
            if u.stop.is_some() {
                return KS10_STOP;
            }
            if address as usize >= u.m.memory.len() {
                return KS10_BAD_ARG;
            }
            u.m.start_at(address);
            pending_stop(u)
        })
    }
}

/// `ks10_uop_next_record` (spec/capi.toml).
///
/// # Safety
/// Each pointer is NULL or valid: `m` from `ks10_uop_new` and not freed,
/// and not used by another thread at the same time; an output pointer to
/// memory of the caller of the size of its type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ks10_uop_next_record(
    m: *mut Ks10Uop,
    max_cycles: u64,
    record: *mut Ks10Record,
) -> c_int {
    if record.is_null() {
        return KS10_NULL;
    }
    let run = |u: &mut Ks10Uop| {
        if u.stop.is_some() {
            return KS10_STOP;
        }
        for _ in 0..max_cycles {
            if let Err(s) = u.m.step() {
                u.stop = Some(s.to_string());
                return KS10_STOP;
            }
            if let Some(r) = u.m.record.take() {
                // SAFETY: `record` is not NULL and points to a ks10_record of
                // the caller (the contract of this function).
                unsafe { record.write(r.into()) };
                return KS10_OK;
            }
        }
        KS10_LIMIT
    };
    // SAFETY: the contract of this function.
    unsafe { with_mut(m, run) }
}

/// `ks10_uop_stop_text` (spec/capi.toml).
///
/// # Safety
/// Each pointer is NULL or valid: `m` from `ks10_uop_new` and not freed,
/// and not used by another thread at the same time; an output pointer to
/// memory of the caller of the size of its type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ks10_uop_stop_text(
    m: *const Ks10Uop,
    text: *mut c_char,
    size: usize,
) -> usize {
    // SAFETY: as in `with_mut`, for reading.
    let Some(u) = (unsafe { m.as_ref() }) else {
        return 0;
    };
    let Some(s) = &u.stop else {
        return 0;
    };
    if !text.is_null() && size > 0 {
        let n = s.len().min(size - 1);
        // SAFETY: `text` points to `size` bytes of the caller, and n + 1 <=
        // size.
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr().cast::<c_char>(), text, n);
            text.add(n).write(0);
        }
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    // NULL handles and NULL output pointers give KS10_NULL, and a bad
    // address or a word of more than 36 bits gives KS10_BAD_ARG.
    #[test]
    fn null_and_bad_arguments() {
        let null = std::ptr::null_mut();
        assert_eq!(unsafe { ks10_uop_master_reset(null) }, KS10_NULL);
        assert_eq!(unsafe { ks10_uop_deposit(null, 0, 0) }, KS10_NULL);
        assert_eq!(unsafe { ks10_uop_examine(null, 0, &mut 0) }, KS10_NULL);
        assert_eq!(unsafe { ks10_uop_start(null, 0) }, KS10_NULL);
        assert_eq!(
            unsafe { ks10_uop_stop_text(null, std::ptr::null_mut(), 0) },
            0
        );
        let mut r = Ks10Record::default();
        assert_eq!(unsafe { ks10_uop_next_record(null, 1, &mut r) }, KS10_NULL);
        let m = ks10_uop_new(0);
        assert!(!m.is_null());
        assert_eq!(
            unsafe { ks10_uop_next_record(m, 1, std::ptr::null_mut()) },
            KS10_NULL
        );
        assert_eq!(
            unsafe { ks10_uop_examine(m, 0, std::ptr::null_mut()) },
            KS10_NULL
        );
        let words = ks10_machine::MEMORY_WORDS as u32;
        assert_eq!(unsafe { ks10_uop_deposit(m, words, 0) }, KS10_BAD_ARG);
        assert_eq!(unsafe { ks10_uop_deposit(m, 0, 1 << 36) }, KS10_BAD_ARG);
        assert_eq!(unsafe { ks10_uop_examine(m, words, &mut 0) }, KS10_BAD_ARG);
        assert_eq!(unsafe { ks10_uop_start(m, words) }, KS10_BAD_ARG);
        // SAFETY: from ks10_uop_new, freed once.
        unsafe { ks10_uop_free(m) };
        // SAFETY: NULL is allowed.
        unsafe { ks10_uop_free(null) };
    }

    // A panic inside a call gives KS10_PANIC, and the machine stays not
    // usable.
    #[test]
    fn a_panic_does_not_cross() {
        let m = ks10_uop_new(0);
        assert_eq!(unsafe { with_mut(m, |_| panic!("test")) }, KS10_PANIC);
        assert_eq!(unsafe { ks10_uop_master_reset(m) }, KS10_PANIC);
        assert_eq!(unsafe { ks10_uop_examine(m, 0, &mut 0) }, KS10_PANIC);
        // SAFETY: from ks10_uop_new, freed once.
        unsafe { ks10_uop_free(m) };
    }

    // The stop text is cut to the buffer and ends with 0.
    #[test]
    fn stop_text() {
        let m = ks10_uop_new(0);
        // SAFETY: from ks10_uop_new.
        unsafe { (*m).stop = Some("abcdef".into()) };
        let mut buf: [c_char; 4] = [0x7f; 4];
        assert_eq!(
            unsafe { ks10_uop_stop_text(m, buf.as_mut_ptr(), buf.len()) },
            6
        );
        assert_eq!(buf.map(|c| u8::from_ne_bytes(c.to_ne_bytes())), *b"abc\0");
        let mut r = Ks10Record::default();
        assert_eq!(unsafe { ks10_uop_next_record(m, 10, &mut r) }, KS10_STOP);
        // SAFETY: from ks10_uop_new, freed once.
        unsafe { ks10_uop_free(m) };
    }
}
