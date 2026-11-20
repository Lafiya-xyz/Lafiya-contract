fn bad(a: i128, b: i128) -> i128 {
    a + b //~ unchecked_amount_arithmetic
}
fn good(a: i128, b: i128) -> Option<i128> {
    a.checked_add(b)
}
fn other_types_ok(a: u32, b: u32) -> u32 { a + b }

// Payout binding delay arithmetic: unchecked `now + delay` traps under
// `overflow-checks = true` instead of returning a typed error.
fn bad_delay(now: u64, delay: u64) -> u64 {
    now + delay //~ unchecked_amount_arithmetic
}

// The checked pattern used by `set_payout_address` must not be flagged.
fn good_delay(now: u64, delay: u64) -> Option<u64> {
    now.checked_add(delay)
}

// Saturating arithmetic is also acceptable and must not be flagged.
fn saturating_delay(now: u64, delay: u64) -> u64 {
    now.saturating_add(delay)
}

fn main() {}