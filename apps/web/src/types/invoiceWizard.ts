/**
 * Type definitions for SME Invoice Submission Wizard (#772)
 * Extended for real IPFS pinning + metadata hash validation (#1001, #1002).
 */

export type WizardStep = 'details' | 'upload' | 'review' | 'mint' | 'confirmation';

export interface InvoiceDetails {
  debtorName: string;
  debtorHash: string;
  amount: bigint;
  currency: string;
  dueDateTimestamp: number;
  description: string;
}

export interface DocumentUpload {
  fileName: string;
  fileSizeBytes: number;
  ipfsCid: string | null;
  status: 'idle' | 'uploading' | 'pinned' | 'error';
  errorMessage?: string;
  /** SHA-256 hex of the pinned content, as committed by the pinning service (#1002). */
  metadataSha256Hex?: string | null;
  /** True when fewer than all pinning providers succeeded (degraded redundancy). */
  degraded?: boolean;
  /** Upload progress percentage 0–100, surfaced by the pinning client. */
  progress?: number;
  /** Transient message shown during slow operations (e.g. polling). */
  progressMessage?: string;
}

export interface InvoiceWizardDraft {
  step: WizardStep;
  details: InvoiceDetails;
  document: DocumentUpload;
  lastSavedAt: number;
}

export interface InvoiceValidationError {
  field: keyof InvoiceDetails;
  message: string;
}

/* ── Metadata hash validation (#1002) ─────────────────────────────────────── */

/**
 * The invoice_nft contract commits `metadata_hash: Bytes` via
 * `commit_metadata_hash`. The web client commits the raw 32-byte SHA-256
 * digest of the pinned document bytes; the hex form is displayed to the user.
 */
export const METADATA_HASH_LENGTH_BYTES = 32;

export interface HashVerificationResult {
  /** Hash the document promised to commit (from pinning service / preview). */
  expectedSha256Hex: string | null;
  /** Hash freshly computed over the pinned bytes as retrieved at verify time. */
  computedSha256Hex: string | null;
  /** True only when both hashes match exactly, case-insensitively. */
  matches: boolean;
}

export interface MintResult {
  invoiceId: string;
  txHash: string;
  mintedAt: number;
  /** The validated SHA-256 hex committed on-chain with the invoice (#1002). */
  metadataHashHex?: string;
  ipfsCid?: string;
}

/** Optional analytics hook fired when a hash verification fails closed. */
export interface HashMismatchEvent {
  fileName: string;
  ipfsCid: string | null;
  expectedSha256Hex: string | null;
  computedSha256Hex: string | null;
  reason: 'preview_mismatch' | 'pinned_content_mismatch' | 'unsupported_webcrypto';
}
