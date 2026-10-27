fn bad(a: i128, b: i128) -> i128 {
    a + b //~ unchecked_amount_arithmetic
}
fn good(a: i128, b: i128) -> Option<i128> {
    a.checked_add(b)
}
fn other_types_ok(a: u32, b: u32) -> u32 { a + b }
fn main() {}
