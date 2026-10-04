// Exposes the source commit to `contractmeta!` (see src/lib.rs). Read only
// from the environment, never from `.git`, so the wasm is a pure function of
// the source tree plus LAFIYA_GIT_COMMIT and stays reproducible.
fn main() {
    println!("cargo:rerun-if-env-changed=LAFIYA_GIT_COMMIT");
    let commit = std::env::var("LAFIYA_GIT_COMMIT").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=LAFIYA_GIT_COMMIT={commit}");
}
