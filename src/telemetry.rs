use std::sync::{Arc, Mutex};
use std::time::Duration;
use crate::models::DiskInfo;
use crate::crypto::Encryptor;

pub fn start_telemetry_thread(
    data: Arc<Mutex<Vec<DiskInfo>>>,
    endpoint: String,
) {
    std::thread::spawn(move || {
        // Spin up a single-threaded tokio runtime inside this thread
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let client = reqwest::Client::builder()
            .use_rustls_tls()               // pure-Rust TLS, no OpenSSL
            .min_tls_version(reqwest::tls::Version::TLS_1_3) // enforce TLS 1.3
            .build()
            .unwrap();

        let enc = Encryptor::from_env();

        loop {
            rt.block_on(async {
                // Snapshot current drive data — hold lock briefly
                let snapshot = {
                    let guard = data.lock().unwrap();
                    guard.clone()
                };

                // Serialize to JSON
                let json = serde_json::json!({
                    "ts": chrono::Utc::now().timestamp(),
                    "drives": snapshot,
                });
                let plaintext = serde_json::to_vec(&json).unwrap();

                // Encrypt
                let ciphertext = enc.encrypt(&plaintext);

                // POST as raw bytes; set content-type so server knows
                let _ = client
                    .post(&endpoint)
                    .header("Content-Type", "application/octet-stream")
                    .body(ciphertext)
                    .send()
                    .await;
                    // Production: handle errors, retry with backoff
            });

            std::thread::sleep(Duration::from_secs(1));
        }
    });
}