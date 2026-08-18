//! B2 instrumentation harness: a small program with raw-pointer
//! dereferences. Built with `FEC_INSTRUMENT=1`, the driver injects a
//! `cementite::__fec_check_deref` call before each raw deref; run, the
//! program must behave identically and report a non-zero check count.

fn main() {
    let x = 42u8;
    let p: *const u8 = &x;
    // raw deref (read)
    let v = unsafe { *p };

    let mut y = 7u8;
    let q: *mut u8 = &mut y;
    // raw deref (write)
    unsafe { *q = v.wrapping_add(1) };
    // raw deref (read)
    let r = unsafe { *q };

    // Program behaviour must be unchanged by instrumentation.
    println!("v={v} y={y} r={r}");

    // Read the count the injected checks accumulated (0 when not
    // instrumented). The at-exit reporter in cementite also prints it.
    println!("fec-count={}", cementite::deref_check_count());
}
