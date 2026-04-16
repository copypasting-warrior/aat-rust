use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose, Engine};
use rand::RngCore;

pub struct Encryptor {
    cipher: Aes256Gcm,
}

impl Encryptor {
    /// Load a 32-byte key from env or a key file — never hardcode
    pub fn from_env() -> Self {
        let key_b64 = std::env::var("TELEMETRY_KEY")
            .expect("TELEMETRY_KEY not set");
        let key_bytes = general_purpose::STANDARD.decode(key_b64).expect("invalid key");
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        Self { cipher: Aes256Gcm::new(key) }
    }

    /// Returns (nonce_bytes || ciphertext) — prepend nonce for easy decryption
    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let mut ciphertext = self.cipher
            .encrypt(nonce, plaintext)
            .expect("encryption failure");

        // Prepend nonce so the server can decrypt
        let mut out = nonce_bytes.to_vec();
        out.append(&mut ciphertext);
        out
    }
}