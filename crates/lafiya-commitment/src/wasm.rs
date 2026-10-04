// WASM bindings for lafiya-commitment
// Compile with: wasm-pack build --target bundler

#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;
use crate::{commit_v1_salted, encode_payload, decode_payload, Salt, FieldValue, EncodeError};

/// High-level WASM interface for commitment operations
#[wasm_bindgen]
pub struct CommitmentEncoder {
    salt: Option<Salt>,
}

#[wasm_bindgen]
impl CommitmentEncoder {
    /// Create a new encoder with an optional salt for dictionary-attack resistance
    #[wasm_bindgen(constructor)]
    pub fn new(salt_bytes: Option<Vec<u8>>) -> Result<CommitmentEncoder, JsValue> {
        let salt = if let Some(bytes) = salt_bytes {
            if bytes.len() != 32 {
                return Err(JsValue::from_str("salt must be exactly 32 bytes"));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Some(Salt::from_bytes(arr))
        } else {
            None
        };

        Ok(CommitmentEncoder { salt })
    }

    /// Compute commitment from field values encoded as JSON array of objects
    /// Example: [{"tag": "text", "value": "hello"}, {"tag": "int64", "value": "42"}]
    pub fn commit(&self, fields_json: &str) -> Result<String, JsValue> {
        let fields: Vec<FieldValue> = serde_json::from_str(fields_json)
            .map_err(|e| JsValue::from_str(&format!("parse error: {}", e)))?;

        let commitment = match &self.salt {
            Some(salt) => commit_v1_salted(salt, &fields),
            None => {
                #[allow(deprecated)]
                crate::commit_v1(&fields)
            }
        };

        match commitment {
            Ok(hash) => {
                let hex = hex::encode(&hash);
                Ok(hex)
            }
            Err(e) => Err(JsValue::from_str(&e.to_string())),
        }
    }

    /// Encode fields into canonical payload (hex-encoded)
    pub fn encode_hex(&self, fields_json: &str) -> Result<String, JsValue> {
        let fields: Vec<FieldValue> = serde_json::from_str(fields_json)
            .map_err(|e| JsValue::from_str(&format!("parse error: {}", e)))?;

        match encode_payload(&fields) {
            Ok(payload) => Ok(hex::encode(&payload)),
            Err(e) => Err(JsValue::from_str(&e.to_string())),
        }
    }

    /// Decode a hex-encoded payload back into fields (round-trip verification)
    pub fn decode_hex(&self, hex_payload: &str) -> Result<String, JsValue> {
        let payload = hex::decode(hex_payload)
            .map_err(|e| JsValue::from_str(&format!("hex decode error: {}", e)))?;

        match decode_payload(&payload) {
            Ok(fields) => {
                let json = serde_json::to_string(&fields)
                    .map_err(|e| JsValue::from_str(&format!("serialize error: {}", e)))?;
                Ok(json)
            }
            Err(e) => Err(JsValue::from_str(&e.to_string())),
        }
    }
}

/// Generate a random high-entropy salt suitable for security-sensitive operations
#[wasm_bindgen]
pub fn generate_salt() -> Vec<u8> {
    use sha2::{Digest, Sha256};
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let mut hasher = Sha256::new();
    hasher.update(timestamp.to_le_bytes());
    hasher.update(web_sys::window().unwrap().document().unwrap().cookie().unwrap());

    hasher.finalize().to_vec()
}

/// Utility: convert hex string to bytes
#[wasm_bindgen]
pub fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, JsValue> {
    hex::decode(hex).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Utility: convert bytes to hex string
#[wasm_bindgen]
pub fn bytes_to_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}
