use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use sha2::{Digest, Sha256};

/// The Domain Separation Tag (DST) prepended to the serialized record payload.
pub const DST: &[u8] = b"Lafiya-Emergency-Record-v1\0";

/// Represents an emergency health record. Contains only the minimal subset
/// of health details critical in emergency triage, plus a salt to prevent guessing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmergencyRecord {
    /// The patient's blood group (e.g., "O+", "A-", or null if unknown)
    pub blood_group: Option<String>,
    /// The patient's genotype (e.g., "AA", "AS", "SS", or null if unknown)
    pub genotype: Option<String>,
    /// List of drug or other critical allergies
    pub allergies: Vec<String>,
    /// List of current medications crucial to note during emergency care
    pub current_medications: Vec<String>,
    /// List of active chronic conditions (e.g. hypertension, diabetes)
    pub chronic_conditions: Vec<String>,
    /// Cryptographically secure random 32-character hex salt (16 bytes of entropy)
    /// to protect low-entropy records from dictionary attacks.
    pub salt: String,
}

impl EmergencyRecord {
    /// Serializes the record using a deterministic JCS-compliant representation
    /// where keys are sorted alphabetically and whitespace is omitted.
    pub fn canonicalize(&self) -> Result<String, serde_json::Error> {
        let mut map = BTreeMap::new();
        map.insert("blood_group", serde_json::to_value(&self.blood_group)?);
        map.insert("genotype", serde_json::to_value(&self.genotype)?);
        map.insert("allergies", serde_json::to_value(&self.allergies)?);
        map.insert("current_medications", serde_json::to_value(&self.current_medications)?);
        map.insert("chronic_conditions", serde_json::to_value(&self.chronic_conditions)?);
        map.insert("salt", serde_json::to_value(&self.salt)?);

        serde_json::to_string(&map)
    }

    /// Computes the 32-byte domain-separated SHA-256 hash commitment of the record.
    pub fn hash(&self) -> Result<[u8; 32], serde_json::Error> {
        let serialized = self.canonicalize()?;
        let mut hasher = Sha256::new();
        hasher.update(DST);
        hasher.update(serialized.as_bytes());
        let result = hasher.finalize();
        let mut hash_bytes = [0u8; 32];
        hash_bytes.copy_from_slice(&result);
        Ok(hash_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonicalization_and_hashing() {
        let record = EmergencyRecord {
            blood_group: Some("O+".to_string()),
            genotype: Some("AA".to_string()),
            allergies: vec!["penicillin".to_string()],
            current_medications: vec!["metformin".to_string()],
            chronic_conditions: vec!["hypertension".to_string()],
            salt: "a4f2b96c8d3e1f0a2b4c6d8e0f1a3b5c".to_string(),
        };

        let canonical = record.canonicalize().unwrap();
        // Check that keys are ordered alphabetically:
        // allergies, blood_group, chronic_conditions, current_medications, genotype, salt
        assert_eq!(
            canonical,
            r#"{"allergies":["penicillin"],"blood_group":"O+","chronic_conditions":["hypertension"],"current_medications":["metformin"],"genotype":"AA","salt":"a4f2b96c8d3e1f0a2b4c6d8e0f1a3b5c"}"#
        );

        let hash_a = record.hash().unwrap();

        // Deserialize from another JSON with different key order, and verify it produces the identical canonical representation
        let json_diff_order = r#"{
            "salt": "a4f2b96c8d3e1f0a2b4c6d8e0f1a3b5c",
            "genotype": "AA",
            "blood_group": "O+",
            "allergies": ["penicillin"],
            "current_medications": ["metformin"],
            "chronic_conditions": ["hypertension"]
        }"#;

        let record_b: EmergencyRecord = serde_json::from_str(json_diff_order).unwrap();
        assert_eq!(record_b.canonicalize().unwrap(), canonical);
        assert_eq!(record_b.hash().unwrap(), hash_a);
    }

    #[test]
    fn test_salt_changes_hash() {
        let record_a = EmergencyRecord {
            blood_group: Some("O+".to_string()),
            genotype: Some("AA".to_string()),
            allergies: vec![],
            current_medications: vec![],
            chronic_conditions: vec![],
            salt: "a4f2b96c8d3e1f0a2b4c6d8e0f1a3b51".to_string(),
        };

        let record_b = EmergencyRecord {
            salt: "a4f2b96c8d3e1f0a2b4c6d8e0f1a3b52".to_string(),
            ..record_a.clone()
        };

        assert_ne!(record_a.hash().unwrap(), record_b.hash().unwrap());
    }
}
