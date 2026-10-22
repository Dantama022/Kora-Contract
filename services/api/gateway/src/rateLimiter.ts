/**
 * API Gateway — Per-key & contract-scoped rate limiter
 *
 * Enforces token-bucket rate limits per API key and per contract identifier:
 * - Each API key has a request budget per window determined by its access tier.
 * - Mutating and sensitive routes can additionally specify a `contractId`,
 *   enforcing contract-scoped limits so a burst on one contract cannot
 *   exhaust capacity across the protocol.
 * - Contract identifiers are validated and normalized before use in bucket keys.
 *
 * Issue #753, Issue #932
 */

import { AccessTier } from "./types";

export const TIER_LIMITS: Record<AccessTier, { maxRequests: number; windowMs: number }> = {
  free: { maxRequests: 60, windowMs: 60_000 },
  standard: { maxRequests: 600, windowMs: 60_000 },
  enterprise: { maxRequests: 6000, windowMs: 60_000 },
};

export const DEFAULT_CONTRACT_LIMIT = {
  maxRequests: 30,
  windowMs: 60_000,
};

interface Bucket {
  count: number;
  windowStart: number;
}

export interface RateLimitResult {
  allowed: boolean;
  remaining: number;
  resetAfterMs: number;
  limitType?: "key" | "contract";
}

/**
 * Validates and normalizes contract identifiers for bucket keys.
 * Soroban contract IDs are typically 56-character base32 strings starting with 'C'
 * or standard alphanumeric identifiers.
 */
export function normalizeContractId(contractId?: string | null): string {
  if (!contractId) return "__unknown_contract__";
  const trimmed = contractId.trim().toUpperCase();
  if (trimmed.length === 0) return "__unknown_contract__";
  // Allow alphanumeric, hyphen, underscore up to 64 chars
  if (/^[A-Z0-9_-]{1,64}$/.test(trimmed)) {
    return trimmed;
  }
  return "__invalid_contract__";
}

export class RateLimiter {
  private buckets = new Map<string, Bucket>();
  private tiers = new Map<string, AccessTier>();
  private contractLimits = new Map<string, { maxRequests: number; windowMs: number }>();
  private defaultContractLimit = { ...DEFAULT_CONTRACT_LIMIT };
  private now: () => number;

  constructor(now: () => number = Date.now, defaultContractMax: number = 30) {
    this.now = now;
    this.defaultContractLimit.maxRequests = defaultContractMax;
  }

  registerKey(key: string, tier: AccessTier): void {
    this.tiers.set(key, tier);
  }

  tierOf(key: string): AccessTier {
    return this.tiers.get(key) ?? "free";
  }

  registerContractLimit(contractId: string, limit: { maxRequests: number; windowMs: number }): void {
    const norm = normalizeContractId(contractId);
    this.contractLimits.set(norm, limit);
  }

  /**
   * Consume request budget.
   * If `contractId` is provided, enforces BOTH key-level limit and contract-scoped limit.
   */
  consume(key: string, contractId?: string): RateLimitResult {
    const t = this.now();

    // 1. Check key-level rate limit
    const tier = this.tierOf(key);
    const keyLimits = TIER_LIMITS[tier];
    let keyBucket = this.buckets.get(key);
    if (!keyBucket || t - keyBucket.windowStart >= keyLimits.windowMs) {
      keyBucket = { count: 0, windowStart: t };
      this.buckets.set(key, keyBucket);
    }

    if (keyBucket.count >= keyLimits.maxRequests) {
      const resetAfterMs = Math.max(0, keyBucket.windowStart + keyLimits.windowMs - t);
      return {
        allowed: false,
        remaining: 0,
        resetAfterMs,
        limitType: "key",
      };
    }

    // 2. Check contract-scoped limit if contractId is provided
    let contractBucket: Bucket | undefined;
    let contractLimitConfig = this.defaultContractLimit;
    let contractBucketKey: string | undefined;

    if (contractId !== undefined) {
      const norm = normalizeContractId(contractId);
      contractBucketKey = `contract:${key}:${norm}`;
      contractLimitConfig = this.contractLimits.get(norm) ?? this.defaultContractLimit;

      contractBucket = this.buckets.get(contractBucketKey);
      if (!contractBucket || t - contractBucket.windowStart >= contractLimitConfig.windowMs) {
        contractBucket = { count: 0, windowStart: t };
        this.buckets.set(contractBucketKey, contractBucket);
      }

      if (contractBucket.count >= contractLimitConfig.maxRequests) {
        const resetAfterMs = Math.max(0, contractBucket.windowStart + contractLimitConfig.windowMs - t);
        return {
          allowed: false,
          remaining: 0,
          resetAfterMs,
          limitType: "contract",
        };
      }
    }

    // Both passed -> increment counts
    keyBucket.count += 1;
    if (contractBucket) {
      contractBucket.count += 1;
    }

    const remaining = contractBucket
      ? Math.min(keyLimits.maxRequests - keyBucket.count, contractLimitConfig.maxRequests - contractBucket.count)
      : keyLimits.maxRequests - keyBucket.count;

    const resetAfterMs = Math.max(0, keyBucket.windowStart + keyLimits.windowMs - t);

    return {
      allowed: true,
      remaining: Math.max(0, remaining),
      resetAfterMs,
    };
  }

  /**
   * Reset bucket(s) for a key. If contractId is specified, resets only that contract bucket.
   */
  reset(key: string, contractId?: string): void {
    if (contractId !== undefined) {
      const norm = normalizeContractId(contractId);
      this.buckets.delete(`contract:${key}:${norm}`);
    } else {
      this.buckets.delete(key);
      // Also delete any contract-scoped buckets for this key
      const prefix = `contract:${key}:`;
      for (const k of Array.from(this.buckets.keys())) {
        if (k.startsWith(prefix)) {
          this.buckets.delete(k);
        }
      }
    }
  }
}
