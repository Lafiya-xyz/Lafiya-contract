struct S; struct P; struct E;
impl E { fn storage(&self) -> S { S } }
impl S { fn persistent(&self) -> P { P } }
impl P { fn set(&self, _k: u32, _v: u32) {} fn extend_ttl(&self, _k: u32, _a: u32, _b: u32) {} }

fn bad(e: &E) {
    e.storage().persistent().set(1, 2); //~ persistent_set_without_ttl
}
fn good(e: &E) {
    e.storage().persistent().set(1, 2);
    e.storage().persistent().extend_ttl(1, 10, 20);
}
#[allow(persistent_set_without_ttl, reason = "TTL is extended by the caller")]
fn allowed(e: &E) {
    e.storage().persistent().set(1, 2);
}
fn main() {}
