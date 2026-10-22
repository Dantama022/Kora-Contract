/**
 * API Gateway — Authentication & Credential Verification
 *
 * Supports API keys and HMAC-SHA256 (HS256) JWTs.
 * Identity is derived strictly from validated credentials, never from
 * unauthenticated client headers alone.
 *
 * Issue #932
 */

import * as crypto from "crypto";
import pino from "pino";
import { AccessTier } from "./types";

const logger = pino({ name: "api-gateway:auth" });

export interface AuthenticatedCaller {
  identity: string;
  authType: "api_key" | "jwt";
  tier: AccessTier;
  isAdmin?: boolean;
}

export interface JwtPayload {
  sub?: string;
  userId?: string;
  apiKey?: string;
  tier?: AccessTier;
  role?: string;
  iss?: string;
  aud?: string | string[];
  exp?: number;
  nbf?: number;
  iat?: number;
  [key: string]: unknown;
}

export interface AuthConfig {
  jwtSecret?: string;
  jwtIssuer?: string;
  jwtAudience?: string;
  validApiKeys?: Map<string, { tier: AccessTier; isAdmin?: boolean; identity: string }>;
  clockSkewSeconds?: number;
}

export function redactSecret(secret?: string): string {
  if (!secret) return "";
  if (secret.length <= 8) return "[REDACTED]";
  return `${secret.slice(0, 4)}...${secret.slice(-4)}[REDACTED]`;
}

export function redactHeaders(headers: Record<string, string | string[] | undefined>): Record<string, unknown> {
  const sanitized: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(headers)) {
    const lower = k.toLowerCase();
    if (lower === "authorization" || lower === "x-api-key" || lower === "cookie") {
      sanitized[k] = "[REDACTED]";
    } else {
      sanitized[k] = v;
    }
  }
  return sanitized;
}

function base64UrlDecode(str: string): string {
  let base64 = str.replace(/-/g, "+").replace(/_/g, "/");
  while (base64.length % 4) {
    base64 += "=";
  }
  return Buffer.from(base64, "base64").toString("utf8");
}

export function createHmacSha256Signature(content: string, secret: string): string {
  return crypto
    .createHmac("sha256", secret)
    .update(content)
    .digest("base64")
    .replace(/=/g, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");
}

export function verifyJwt(token: string, config: AuthConfig, nowMs: number = Date.now()): { valid: boolean; payload?: JwtPayload; error?: string } {
  const secret = config.jwtSecret ?? process.env.JWT_SECRET;
  if (!secret) {
    logger.warn("JWT verification attempted but JWT_SECRET is not configured");
    return { valid: false, error: "JWT secret not configured" };
  }

  const parts = token.split(".");
  if (parts.length !== 3) {
    logger.info({ reason: "malformed_token_structure" }, "JWT validation failed");
    return { valid: false, error: "Invalid token structure" };
  }

  const [headerB64, payloadB64, signatureB64] = parts;

  // Verify header
  let header: { alg?: string; typ?: string };
  try {
    header = JSON.parse(base64UrlDecode(headerB64!));
  } catch {
    logger.info({ reason: "malformed_header" }, "JWT validation failed");
    return { valid: false, error: "Invalid token header" };
  }

  if (header.alg !== "HS256") {
    logger.info({ reason: "unsupported_algorithm" }, "JWT validation failed");
    return { valid: false, error: "Unsupported algorithm" };
  }

  // Verify signature
  const expectedSig = createHmacSha256Signature(`${headerB64}.${payloadB64}`, secret);
  const sigBuffer = Buffer.from(signatureB64!);
  const expectedBuffer = Buffer.from(expectedSig);

  if (sigBuffer.length !== expectedBuffer.length || !crypto.timingSafeEqual(sigBuffer, expectedBuffer)) {
    logger.info({ reason: "signature_mismatch" }, "JWT validation failed");
    return { valid: false, error: "Signature mismatch" };
  }

  // Parse payload
  let payload: JwtPayload;
  try {
    payload = JSON.parse(base64UrlDecode(payloadB64!));
  } catch {
    logger.info({ reason: "malformed_payload" }, "JWT validation failed");
    return { valid: false, error: "Invalid token payload" };
  }

  const nowSec = Math.floor(nowMs / 1000);
  const skew = config.clockSkewSeconds ?? 30;

  // Check expiry
  if (typeof payload.exp === "number" && nowSec >= payload.exp + skew) {
    logger.info({ reason: "token_expired" }, "JWT validation failed");
    return { valid: false, error: "Token expired" };
  }

  // Check not before
  if (typeof payload.nbf === "number" && nowSec < payload.nbf - skew) {
    logger.info({ reason: "token_not_yet_valid" }, "JWT validation failed");
    return { valid: false, error: "Token not yet valid" };
  }

  // Check issuer if configured
  const expectedIss = config.jwtIssuer ?? process.env.JWT_ISSUER;
  if (expectedIss && payload.iss !== expectedIss) {
    logger.info({ reason: "issuer_mismatch" }, "JWT validation failed");
    return { valid: false, error: "Invalid issuer" };
  }

  // Check audience if configured
  const expectedAud = config.jwtAudience ?? process.env.JWT_AUDIENCE;
  if (expectedAud) {
    const tokenAud = payload.aud;
    const matchesAud = Array.isArray(tokenAud) ? tokenAud.includes(expectedAud) : tokenAud === expectedAud;
    if (!matchesAud) {
      logger.info({ reason: "audience_mismatch" }, "JWT validation failed");
      return { valid: false, error: "Invalid audience" };
    }
  }

  return { valid: true, payload };
}

export class Authenticator {
  private apiKeys = new Map<string, { tier: AccessTier; isAdmin?: boolean; identity: string }>();

  constructor(private config: AuthConfig = {}) {
    // Load configured keys from config
    if (config.validApiKeys) {
      for (const [k, v] of config.validApiKeys.entries()) {
        this.apiKeys.set(k, v);
      }
    }
    // Load from environment: GATEWAY_API_KEYS="key1:free,key2:enterprise"
    const envKeys = process.env.GATEWAY_API_KEYS ?? "";
    for (const entry of envKeys.split(",").map((s) => s.trim()).filter(Boolean)) {
      const [key, tier] = entry.split(":");
      if (key) {
        const accessTier: AccessTier = tier === "enterprise" || tier === "standard" ? tier : "free";
        this.apiKeys.set(key, {
          tier: accessTier,
          identity: `apikey:${key.slice(0, 8)}`,
        });
      }
    }

    // Load admin keys: GATEWAY_ADMIN_KEYS="adminkey1,adminkey2"
    const envAdminKeys = process.env.GATEWAY_ADMIN_KEYS ?? "";
    for (const key of envAdminKeys.split(",").map((s) => s.trim()).filter(Boolean)) {
      this.apiKeys.set(key, {
        tier: "enterprise",
        isAdmin: true,
        identity: `admin:${key.slice(0, 8)}`,
      });
    }
  }

  registerKey(key: string, tier: AccessTier, identity?: string, isAdmin: boolean = false): void {
    this.apiKeys.set(key, {
      tier,
      isAdmin,
      identity: identity ?? `apikey:${key.slice(0, 8)}`,
    });
  }

  authenticate(headers: Record<string, string | string[] | undefined>, nowMs: number = Date.now()): AuthenticatedCaller | null {
    const authHeader = headers["authorization"] || headers["Authorization"];
    const apiKeyHeader = headers["x-api-key"] || headers["X-Api-Key"];

    // 1. Try Bearer JWT
    if (typeof authHeader === "string" && authHeader.startsWith("Bearer ")) {
      const token = authHeader.slice(7).trim();
      const res = verifyJwt(token, this.config, nowMs);
      if (res.valid && res.payload) {
        const payload = res.payload;
        const identity = payload.sub || payload.userId || (payload.apiKey ? `apikey:${payload.apiKey.slice(0, 8)}` : "jwt_user");
        const tier = payload.tier ?? "standard";
        const isAdmin = payload.role === "admin";
        return { identity, authType: "jwt", tier, isAdmin };
      }
      return null;
    }

    // 2. Try API key (from X-Api-Key or Authorization header)
    let candidateKey: string | undefined;
    if (typeof apiKeyHeader === "string") {
      candidateKey = apiKeyHeader.trim();
    } else if (typeof authHeader === "string" && authHeader.startsWith("ApiKey ")) {
      candidateKey = authHeader.slice(7).trim();
    }

    if (candidateKey) {
      const keyInfo = this.apiKeys.get(candidateKey);
      if (keyInfo) {
        return {
          identity: keyInfo.identity,
          authType: "api_key",
          tier: keyInfo.tier,
          isAdmin: keyInfo.isAdmin,
        };
      }
      logger.info({ reason: "unknown_api_key", keyPrefix: redactSecret(candidateKey) }, "Authentication failed for API key");
      return null;
    }

    logger.info({ reason: "missing_credentials" }, "Authentication failed: no credentials supplied");
    return null;
  }
}
