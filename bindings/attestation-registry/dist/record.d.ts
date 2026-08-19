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
export declare function canonicalizeRecord(record: EmergencyRecord): string;
/**
 * Computes the 32-byte domain-separated SHA-256 hash commitment of an EmergencyRecord.
 */
export declare function hashRecord(record: EmergencyRecord): Buffer;
