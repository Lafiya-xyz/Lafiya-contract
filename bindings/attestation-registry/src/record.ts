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

/**
 * Record classes that may carry a per-class quorum policy.
 *
 * Mirrors the on-chain `RecordClass` enum in the attestation-registry contract.
 */
export enum RecordClass {
  BloodGroup = "BloodGroup",
  Genotype = "Genotype",
  Allergies = "Allergies",
  CurrentMedications = "CurrentMedications",
  ChronicConditions = "ChronicConditions",
}

/**
 * Quorum policy for a record class.
 *
 * `required` is the number of *distinct* attesters that must have co-attested
 * before the record is reported as verified. `required` of 1 is the legacy
 * single-attester behaviour.
 */
export interface QuorumPolicy {
  record_class: RecordClass;
  required: number;
}

/**
 * Aggregated quorum status for a single record commitment.
 *
 * `distinct_attesters` counts unique attester addresses only; repeated
 * attestations from the same attester do not increase the count.
 */
export interface QuorumStatus {
  record_hash: string;
  record_class: RecordClass;
  required: number;
  distinct_attesters: number;
  satisfied: boolean;
}

/**
 * Dedicated verification states surfaced to verifiers.
 *
 * - `Verified`: quorum policy satisfied and no active dispute.
 * - `Pending`: quorum policy not yet satisfied.
 * - `Disputed`: patient-initiated dispute overlay is active; verifiers MUST
 *   surface this state and MUST NOT report the record as verified.
 * - `Revoked`: the attestation has been revoked by an attester or admin.
 * - `Superseded`: a newer attestation for the same record class exists.
 * - `Unknown`: no attestation found for the commitment.
 */
export enum VerificationState {
  Verified = "Verified",
  Pending = "Pending",
  Disputed = "Disputed",
  Revoked = "Revoked",
  Superseded = "Superseded",
  Unknown = "Unknown",
}

/**
 * Full verification view returned by the registry.
 *
 * Contains only public data: no plaintext record fields are included.
 */
export interface VerificationView {
  state: VerificationState;
  quorum: QuorumStatus;
  /** True when an active patient dispute overlay exists for this record. */
  disputed: boolean;
  /** Optional status-list inclusion proof reference, if available. */
  status_list?: StatusListInclusion;
}

/**
 * Patient-initiated dispute overlay entry.
 *
 * Keyed by `(record_hash, signer)`. The signature is produced with the
 * patient signature scheme from the QR spec and is domain-separated to
 * prevent replay across records or contracts.
 */
export interface DisputeOverlay {
  record_hash: string;
  signer: string;
  /** Domain-separated signature over the dispute commitment. */
  signature: string;
  /** Ledger sequence at which the dispute was recorded. */
  created_at: number;
  /** True once an attester or admin has cleared the dispute. */
  cleared: boolean;
}

/**
 * Inclusion proof for a record within a versioned status-list segment.
 *
 * Aligns with W3C Bitstring Status List semantics: the verifier validates
 * the proof against a signed root for the segment version.
 */
export interface StatusListInclusion {
  /** Identifier of the status-list segment. */
  segment_id: string;
  /** Monotonic version of the segment commitment. */
  version: number;
  /** Bit index within the segment's bitstring. */
  index: number;
  /** Bit value at `index` (true = revoked/invalid). */
  value: boolean;
  /** Signed root commitment for the segment version. */
  signed_root: string;
  /** Merkle/bitstring inclusion proof nodes (hex-encoded). */
  proof: string[];
}