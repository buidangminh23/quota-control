use std::time::Instant;

use chrono::Utc;
use serde_json::json;
use uc_logscan::context::ContextWindows;

fn main() {
    let mut reader = ContextWindows::from_environment();
    for pass in ["initial", "cached"] {
        let started = Instant::now();
        let sessions = reader.scan(Utc::now());
        let stats = reader.stats();
        println!(
            "{}",
            json!({
                "pass":pass,
                "seconds":started.elapsed().as_secs_f64(),
                "filesRead":stats.files_read,
                "filesCached":stats.files_cached,
                "bytesRead":stats.bytes_read,
                "sessions":sessions,
            })
        );
    }
}
