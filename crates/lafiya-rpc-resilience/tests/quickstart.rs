//! Integration test against a local `stellar/quickstart` node: with the
//! primary RPC URL pointing at a dead port, a real signed transaction still
//! lands through the secondary.
//!
//! Ignored by default because it needs Docker and the `stellar` CLI:
//!
//! ```bash
//! docker run -d -p 8000:8000 stellar/quickstart --local --enable rpc
//! cargo test -p lafiya-rpc-resilience --test quickstart -- --ignored
//! ```
//!
//! Set `LAFIYA_QUICKSTART_RPC_URL` to use a node other than
//! `http://localhost:8000/rpc`.

use lafiya_rpc_resilience::http::HttpRpcProvider;
use lafiya_rpc_resilience::{
    FailoverClient, RecoveryLog, RecoveryResult, RetryPolicy, RpcProvider, SignedTx,
};
use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;

const PASSPHRASE: &str = "Standalone Network ; February 2017";

fn stellar(args: &[&str], stdin: Option<&str>) -> String {
    let mut child = Command::new("stellar")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("stellar CLI on PATH");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "stellar {args:?} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
#[ignore = "needs a local stellar/quickstart node and the stellar CLI"]
fn dead_primary_fails_over_to_quickstart_secondary() {
    let rpc = std::env::var("LAFIYA_QUICKSTART_RPC_URL")
        .unwrap_or_else(|_| "http://localhost:8000/rpc".into());
    let net = ["--rpc-url", &rpc, "--network-passphrase", PASSPHRASE];
    let key = format!("lafiya-failover-{}", std::process::id());

    stellar(
        &[
            &["keys", "generate", &key, "--fund", "--overwrite"],
            &net[..],
        ]
        .concat(),
        None,
    );
    let address = stellar(&["keys", "address", &key], None);
    let unsigned = stellar(
        &[
            &[
                "tx",
                "new",
                "payment",
                "--source",
                &key,
                "--destination",
                &address,
                "--amount",
                "1",
                "--build-only",
            ],
            &net[..],
        ]
        .concat(),
        None,
    );
    let envelope = stellar(
        &[&["tx", "sign", "--sign-with-key", &key], &net[..]].concat(),
        Some(&unsigned),
    );
    let hash = stellar(&[&["tx", "hash"], &net[..]].concat(), Some(&envelope));

    let dead_port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let providers: Vec<Box<dyn RpcProvider>> = vec![
        Box::new(HttpRpcProvider::new(format!(
            "http://127.0.0.1:{dead_port}"
        ))),
        Box::new(HttpRpcProvider::new(rpc.clone())),
    ];
    let policy = RetryPolicy {
        max_poll_rounds: 10,
        base_backoff: Duration::from_millis(500),
        ..RetryPolicy::default()
    };
    let mut client = FailoverClient::new(providers, policy).with_sleep(std::thread::sleep);
    let mut log = RecoveryLog::new();
    let result = client.submit_with_recovery(&SignedTx::new(&hash, envelope), &mut log);

    match result {
        RecoveryResult::Accepted { provider, .. } => assert_eq!(provider, rpc),
        other => panic!("{other:?}\n{:#?}", log.lines()),
    }
}
