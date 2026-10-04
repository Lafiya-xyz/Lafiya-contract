import { hash } from "@stellar/stellar-sdk";
import { Buffer } from "buffer";

/**
 * Interface representing an emergency record structure in Lafiya.
 */
export interface EmergencyRecord {
  blood_group: string | null;
  genotype: string | null;
  allergies: string[];
  current_medications: string[];
  chronic_conditions: string[];
  salt: string;
}

/**
 * Serializes an EmergencyRecord deterministically (JCS-compliant layout,
 * keys sorted alphabetically, zero whitespace).
 */
export function canonicalizeRecord(record: EmergencyRecord): string {
  const sorted = {
    allergies: record.allergies || [],
    blood_group: record.blood_group === undefined ? null : record.blood_group,
    chronic_conditions: record.chronic_conditions || [],
    current_medications: record.current_medications || [],
    genotype: record.genotype === undefined ? null : record.genotype,
    salt: record.salt,
  };
  return JSON.stringify(sorted);
}

/**
 * Computes the 32-byte domain-separated SHA-256 hash commitment of an EmergencyRecord.
 */
export function hashRecord(record: EmergencyRecord): Buffer {
  const dst = Buffer.concat([Buffer.from("Lafiya-Emergency-Record-v1"), Buffer.from([0])]);
  const serialized = canonicalizeRecord(record);
  const payload = Buffer.concat([dst, Buffer.from(serialized, "utf-8")]);
  return hash(payload);
}
