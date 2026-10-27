//! Structured logging for operator tooling.
//!
//! Human-readable output by default, `--log-format json` for machines, and
//! `-v`/`-vv` for more detail. Every byte written by the subscriber passes
//! through [`redact`], so secret seeds can never reach a log sink even if a
//! call site logs one by mistake.

use std::io::{self, Write};
use tracing_subscriber::{fmt::MakeWriter, EnvFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LogFormat {
    Text,
    Json,
}

const REDACTED: &str = "[REDACTED]";

/// Field names whose values are always replaced with `[REDACTED]`, regardless
/// of whether the value itself looks like a known secret format. Covers
/// passphrases, keyring identifiers, and external-signer JSON payloads.
const SENSITIVE_KEYS: &[&str] = &[
    "passphrase",
    "password",
    "secret",
    "seed",
    "mnemonic",
    "private_key",
    "privatekey",
    "secret_key",
    "secretkey",
    "keyring",
    "keyring_id",
    "keyringid",
    "keyring_entry",
    "signer_payload",
    "signerpayload",
    "signer_request",
    "signerrequest",
    "signer_response",
    "signerresponse",
    "payload",
];

fn is_sensitive_key(key: &str) -> bool {
    let k = key.trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c.is_whitespace());
    let k = k.to_ascii_lowercase();
    SENSITIVE_KEYS.iter().any(|s| k == *s)
}

/// Replace anything that looks like a secret with `[REDACTED]`:
/// Stellar secret seeds (`S` + 55 base32 chars), BIP-39 style mnemonic
/// phrases (12 or more consecutive lowercase words), and values assigned to
/// sensitive keys (`passphrase=...`, `"secret_key": "..."`, etc.).
pub fn redact(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut token = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            token.push(ch);
        } else {
            flush_token(&mut token, &mut out);
            out.push(ch);
        }
    }
    flush_token(&mut token, &mut out);
    let out = redact_sensitive_assignments(&out);
    redact_mnemonics(&out)
}

/// Redact the value following `key=`, `key:`, or `"key": "..."` when the key
/// is sensitive. Handles both shell-style and JSON-style assignments.
fn redact_sensitive_assignments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Find the start of a potential key token.
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if i == start {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        let key = &input[start..i];
        // Look ahead for an assignment separator.
        let mut j = i;
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        let sep = bytes.get(j).copied();
        if is_sensitive_key(key) && matches!(sep, Some(b'=') | Some(b':')) {
            out.push_str(key);
            out.push(sep.unwrap() as char);
            j += 1;
            // Skip whitespace and an opening quote.
            while j < bytes.len() && bytes[j] == b' ' {
                out.push(' ');
                j += 1;
            }
            let quote = match bytes.get(j).copied() {
                Some(q @ (b'"' | b'\'')) => {
                    out.push(q as char);
                    j += 1;
                    Some(q)
                }
                _ => None,
            };
            // Consume the value up to the closing quote or delimiter.
            while j < bytes.len() {
                let b = bytes[j];
                if let Some(q) = quote {
                    if b == q {
                        break;
                    }
                } else if b == b',' || b == b' ' || b == b'\n' || b == b'\t' {
                    break;
                }
                j += 1;
            }
            out.push_str(REDACTED);
            if let Some(q) = quote {
                if j < bytes.len() {
                    out.push(q as char);
                    j += 1;
                }
            }
            i = j;
        } else {
            out.push_str(key);
            i = j;
        }
    }
    out
}

fn is_secret_seed(token: &str) -> bool {
    token.len() == 56
        && token.starts_with('S')
        && token
            .bytes()
            .all(|b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b))
}

fn flush_token(token: &mut String, out: &mut String) {
    if is_secret_seed(token) {
        out.push_str(REDACTED);
    } else {
        out.push_str(token);
    }
    token.clear();
}

fn redact_mnemonics(input: &str) -> String {
    const MIN_WORDS: usize = 12;
    let words: Vec<&str> = input.split(' ').collect();
    let is_word = |w: &str| (3..=8).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_lowercase());
    let mut out: Vec<&str> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let run = words[i..].iter().take_while(|w| is_word(w)).count();
        if run >= MIN_WORDS {
            out.push(REDACTED);
            i += run;
        } else {
            out.push(words[i]);
            i += 1;
        }
    }
    out.join(" ")
}

/// Writer that redacts each buffered write before forwarding it.
pub struct RedactingWriter<W: Write>(pub W);

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.0.write_all(redact(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[derive(Clone, Copy)]
struct StderrRedacted;

impl<'a> MakeWriter<'a> for StderrRedacted {
    type Writer = RedactingWriter<io::Stderr>;
    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter(io::stderr())
    }
}

fn filter(verbosity: u8) -> EnvFilter {
    let level = match verbosity {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    EnvFilter::try_from_env("LAFIYA_LOG").unwrap_or_else(|_| EnvFilter::new(level))
}

/// Keeps exporters alive until the process exits.
pub struct LogGuard {
    #[cfg(feature = "otel")]
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        #[cfg(feature = "otel")]
        if let Some(p) = self.provider.take() {
            let _ = p.shutdown();
        }
    }
}

/// Install the global subscriber. Logs go to stderr so stdout stays usable
/// for command output (`config env`, `auth decode --format json`).
pub fn init(format: LogFormat, verbosity: u8) -> LogGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;

    let fmt_layer = match format {
        LogFormat::Text => tracing_subscriber::fmt::layer()
            .with_writer(StderrRedacted)
            .with_target(false)
            .boxed(),
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_writer(StderrRedacted)
            .boxed(),
    };
    let registry = tracing_subscriber::registry()
        .with(filter(verbosity))
        .with(fmt_layer);

    #[cfg(feature = "otel")]
    {
        let provider = otel_provider();
        match &provider {
            Some(p) => {
                use opentelemetry::trace::TracerProvider as _;
                let tracer = p.tracer("lafiya-cli");
                let _ = registry
                    .with(tracing_opentelemetry::layer().with_tracer(tracer))
                    .try_init();
            }
            None => {
                let _ = registry.try_init();
            }
        }
        LogGuard { provider }
    }
    #[cfg(not(feature = "otel"))]
    {
        let _ = registry.try_init();
        LogGuard {}
    }
}

/// OTLP/HTTP exporter, enabled when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
#[cfg(feature = "otel")]
fn otel_provider() -> Option<opentelemetry_sdk::trace::SdkTracerProvider> {
    std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT")?;
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
        .map_err(|e| eprintln!("OTLP exporter disabled: {e}"))
        .ok()?;
    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name("lafiya-cli")
        .build();
    Some(
        opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_simple_exporter(exporter)
            .with_resource(resource)
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const SEED: &str = "SBZVMB74Z76QZ3ZOY7UTDFYKMEGKW5XFJEB6PFKBF4UYSSWHG4EDH7PY";

    #[test]
    fn redacts_secret_seeds() {
        let line = format!("signing with {SEED} now");
        let out = redact(&line);
        assert!(!out.contains(SEED));
        assert_eq!(out, "signing with [REDACTED] now");
    }

    #[test]
    fn keeps_public_keys_and_contract_ids() {
        let g = "GBZXN7PIRZGNMHGA7MUUUF4GWPY5AYPV6LY4UV2GL6VJGIQRXFDNMADI";
        let c = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";
        assert_eq!(redact(g), g);
        assert_eq!(redact(c), c);
    }

    #[test]
    fn redacts_mnemonic_phrases() {
        let phrase =
            "abandon ability able about above absent absorb abstract absurd abuse access accident";
        let out = redact(&format!("seed: {phrase} (from vault)"));
        assert_eq!(out, "seed: [REDACTED] (from vault)");
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = RedactingWriter<Capture>;
        fn make_writer(&'a self) -> Self::Writer {
            RedactingWriter(self.clone())
        }
    }

    #[test]
    fn json_logs_are_valid_and_redacted() {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(capture.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("attester.add", network = "testnet");
            let _e = span.enter();
            tracing::info!(secret = SEED, "submitting");
        });
        let bytes = capture.0.lock().unwrap().clone();
        let line = String::from_utf8(bytes).unwrap();
        let json: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(json["fields"]["message"], "submitting");
        assert_eq!(json["fields"]["secret"], "[REDACTED]");
        assert_eq!(json["span"]["name"], "attester.add");
        assert!(!line.contains(SEED));
    }

    #[test]
    fn redacts_passphrase_assignments() {
        let out = redact("unlocking keyring passphrase=hunter2 now");
        assert!(!out.contains("hunter2"));
        assert_eq!(out, "unlocking keyring passphrase=[REDACTED] now");
    }

    #[test]
    fn redacts_json_passphrase_and_keyring_fields() {
        let line = r#"{"passphrase":"correct horse","keyring_id":"lafiya/signer","network":"testnet"}"#;
        let out = redact(line);
        assert!(!out.contains("correct horse"));
        assert!(!out.contains("lafiya/signer"));
        assert!(out.contains("\"network\":\"testnet\""));
    }

    #[test]
    fn redacts_external_signer_payloads() {
        let line = r#"signer_request={"payload":"deadbeefcafe","op":"sign"}"#;
        let out = redact(line);
        assert!(!out.contains("deadbeefcafe"));
        assert!(out.contains("\"op\":\"sign\""));
    }

    #[test]
    fn redacts_raw_key_bytes_hex() {
        // A 32-byte key rendered as hex must not survive redaction when it
        // appears under a sensitive key.
        let hex = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
        let out = redact(&format!("private_key={hex}"));
        assert!(!out.contains(hex));
        assert_eq!(out, "private_key=[REDACTED]");
    }

    #[test]
    fn text_logs_never_leak_secret_material() {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .finish();
        let phrase =
            "abandon ability able about above absent absorb abstract absurd abuse access accident";
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(secret = SEED, "seed");
            tracing::info!(passphrase = "hunter2", "unlock");
            tracing::info!(mnemonic = phrase, "phrase");
            tracing::info!("raw {} and {}", SEED, phrase);
        });
        let line = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(!line.contains(SEED));
        assert!(!line.contains("hunter2"));
        assert!(!line.contains("abandon ability"));
    }

    #[test]
    fn json_logs_never_leak_secret_material() {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(capture.clone())
            .finish();
        let phrase =
            "abandon ability able about above absent absorb abstract absurd abuse access accident";
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(passphrase = "hunter2", keyring_id = "lafiya/signer", "unlock");
            tracing::info!(mnemonic = phrase, "phrase");
        });
        let line = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(!line.contains("hunter2"));
        assert!(!line.contains("lafiya/signer"));
        assert!(!line.contains("abandon ability"));
        for l in line.lines() {
            let _: serde_json::Value = serde_json::from_str(l).unwrap();
        }
    }

    #[test]
    fn panic_messages_are_redacted_by_writer() {
        // Panics are formatted by the default hook and written to stderr; the
        // redacting writer is what protects log sinks. Here we assert the
        // redaction function itself scrubs a panic-style message.
        let msg = format!("assertion failed: seed == {SEED}");
        assert!(!redact(&msg).contains(SEED));
    }
}
