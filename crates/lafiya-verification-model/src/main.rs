//! Differential-testing adapter: reads one model input per line (JSON) on
//! stdin and prints one `{"verdict","reason","qualifier"}` object per line.

use std::io::{self, BufRead, BufWriter, Write};

use lafiya_verification_model::{evaluate, Input};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let input: Input = serde_json::from_str(&line)?;
        serde_json::to_writer(&mut out, &evaluate(&input))?;
        out.write_all(b"\n")?;
    }
    out.flush()?;
    Ok(())
}
