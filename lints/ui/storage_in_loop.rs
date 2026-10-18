struct S; struct P; struct E;
impl E { fn storage(&self) -> S { S } }
impl S { fn persistent(&self) -> P { P } }
impl P { fn get(&self, _k: u32) -> u32 { 0 } }

fn bad(e: &E) {
    for i in 0..3 { let _ = e.storage().persistent().get(i); } //~ storage_in_loop
}
fn good(e: &E) {
    let _ = e.storage().persistent().get(0);
}
fn main() {}
