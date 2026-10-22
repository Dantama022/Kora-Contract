import { ApiGateway, RateLimitedError } from "../src/gateway";
import { createMemoryStore } from "../src/store";
import { RateLimiter, normalizeContractId } from "../src/rateLimiter";
import { Authenticator, createHmacSha256Signature, redactSecret, redactHeaders } from "../src/auth";
import * as http from "http";

function seedStore() {
  return createMemoryStore({
    lastIndexedLedger: 12345,
    invoices: [
      { id: "1", sme: "SME_A", debtorHash: "h1", amount: "100", currency: "USDC", dueDate: 10, ipfsCid: "cid1", riskScore: 20, riskTier: "AAA", status: "Listed", createdAt: 1 },
      { id: "2", sme: "SME_A", debtorHash: "h2", amount: "200", currency: "USDC", dueDate: 11, ipfsCid: "cid2", riskScore: 70, riskTier: "B", status: "Funded", createdAt: 2 },
      { id: "3", sme: "SME_B", debtorHash: "h3", amount: "300", currency: "EURC", dueDate: 12, ipfsCid: "cid3", riskScore: 90, riskTier: "C", status: "Listed", createdAt: 3 },
    ],
    listings: [
      { invoiceId: "1", seller: "SME_A", askingPrice: "90", faceValue: "100", token: "USDC", fundedAmount: "10", fundingDeadline: 99, isActive: true },
      { invoiceId: "2", seller: "SME_A", askingPrice: "180", faceValue: "200", token: "USDC", fundedAmount: "200", fundingDeadline: 99, isActive: false },
    ],
    positions: [
      { investor: "INV_1", invoiceId: "1", contributed: "10", shareBps: 1000, yieldClaimed: "0" },
      { investor: "INV_2", invoiceId: "1", contributed: "20", shareBps: 2000, yieldClaimed: "0" },
    ],
    riskScores: [
      { subject: "SME_A", kind: "sme", score: 25, verifier: "V1", updatedAt: 5 },
      { subject: "h1", kind: "debtor", score: 40, verifier: "V1", updatedAt: 6 },
    ],
  });
}

function makeJwt(payload: Record<string, unknown>, secret: string): string {
  const header = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url");
  const pl = Buffer.from(JSON.stringify(payload)).toString("base64url");
  const sig = createHmacSha256Signature(`${header}.${pl}`, secret);
  return `${header}.${pl}.${sig}`;
}

// Mock HTTP helper
function simulateRequest(
  gateway: ApiGateway,
  method: string,
  url: string,
  headers: Record<string, string> = {},
  body?: unknown,
): Promise<{ statusCode: number; headers: Record<string, string | string[] | undefined>; body: any }> {
  return new Promise((resolve, reject) => {
    const rawReq = new (require("stream").Readable)();
    rawReq.method = method;
    rawReq.url = url;
    rawReq.headers = headers;

    const payload = body !== undefined ? JSON.stringify(body) : "";
    rawReq.push(payload);
    rawReq.push(null);

    const resHeaders: Record<string, string | string[]> = {};
    let statusCode = 200;
    let resData = "";

    const rawRes: any = {
      writeHead: (status: number, hdrs: Record<string, string>) => {
        statusCode = status;
        if (hdrs) Object.assign(resHeaders, hdrs);
      },
      setHeader: (name: string, val: string) => {
        resHeaders[name] = val;
      },
      end: (chunk?: string) => {
        if (chunk) resData += chunk;
        let parsed: any;
        try {
          parsed = JSON.parse(resData);
        } catch {
          parsed = resData;
        }
        resolve({ statusCode, headers: resHeaders, body: parsed });
      },
    };

    gateway.handleHttpRequest(rawReq as any, rawRes as any).catch(reject);
  });
}

describe("api-gateway (issue #753 & #932)", () => {
  // ── Existing read-query tests ──────────────────────────────────────────────
  test("invoices support filter combinations, sorting, and cursor pagination", () => {
    const gw = new ApiGateway(seedStore());
    const page1 = gw.queryInvoices(
      { sme: { eq: "SME_A" } },
      { limit: 1 },
      { apiKey: "k1", sortBy: "riskScore", sortDir: "desc" },
    );
    expect(page1.data).toHaveLength(1);
    expect(page1.data[0].id).toBe("2");
    expect(page1.lastIndexedLedger).toBe(12345);
    expect(page1.nextCursor).not.toBeNull();

    const page2 = gw.queryInvoices(
      { sme: { eq: "SME_A" } },
      { limit: 1, cursor: page1.nextCursor as string },
      { apiKey: "k1", sortBy: "riskScore", sortDir: "desc" },
    );
    expect(page2.data[0].id).toBe("1");
    expect(page2.nextCursor).toBeNull();

    const filtered = gw.queryInvoices(
      { status: { eq: "Listed" }, riskTier: { in: ["C"] } },
      {},
      { apiKey: "k1" },
    );
    expect(filtered.data.map((i) => i.id)).toEqual(["3"]);
  });

  test("listings, positions, risk scores query correctly with freshness field", () => {
    const gw = new ApiGateway(seedStore());
    expect(
      gw.queryListings({ isActive: true }, {}, { apiKey: "k" }).data.map((l) => l.invoiceId),
    ).toEqual(["1"]);
    expect(
      gw.queryPositions({ investor: { eq: "INV_2" } }, {}, { apiKey: "k" }).data,
    ).toHaveLength(1);
    const rs = gw.queryRiskScores({ kind: "debtor" }, {}, { apiKey: "k" });
    expect(rs.data).toHaveLength(1);
    expect(rs.lastIndexedLedger).toBe(12345);
  });

  test("rate-limit enforcement blocks over-budget keys and reset restores", () => {
    let now = 0;
    const limiter = new RateLimiter(() => now);
    limiter.registerKey("free-key", "free");
    const gw = new ApiGateway(seedStore(), limiter);
    // free tier = 60/min: exhaust it.
    for (let i = 0; i < 60; i++) {
      gw.queryInvoices({}, {}, { apiKey: "free-key" });
    }
    expect(() => gw.queryInvoices({}, {}, { apiKey: "free-key" })).toThrow(RateLimitedError);
    // Window elapses → budget restored.
    now += 60_000;
    expect(() => gw.queryInvoices({}, {}, { apiKey: "free-key" })).not.toThrow();
    // Explicit reset also restores.
    for (let i = 0; i < 59; i++) gw.queryInvoices({}, {}, { apiKey: "free-key" });
    limiter.reset("free-key");
    expect(() => gw.queryInvoices({}, {}, { apiKey: "free-key" })).not.toThrow();
  });

  test("unknown keys default to free tier; higher tiers allow more", () => {
    const limiter = new RateLimiter();
    limiter.registerKey("ent", "enterprise");
    const gw = new ApiGateway(seedStore(), limiter);
    for (let i = 0; i < 61; i++) {
      if (i < 60) gw.queryInvoices({}, {}, { apiKey: "anon" });
      else expect(() => gw.queryInvoices({}, {}, { apiKey: "anon" })).toThrow(RateLimitedError);
    }
    // enterprise budget is far larger — 61 requests must all pass.
    for (let i = 0; i < 61; i++) gw.queryInvoices({}, {}, { apiKey: "ent" });
  });

  // ── Issue #932 Rate Limiter & Contract Scoping Tests ────────────────────────
  test("contract-scoped rate limiting isolates different contracts", () => {
    let now = 1000;
    const limiter = new RateLimiter(() => now, 2); // 2 req/min contract limit
    limiter.registerKey("partner", "standard"); // 600 req/min key budget

    // 2 requests for contract A succeed
    expect(limiter.consume("partner", "CONTRACT_A").allowed).toBe(true);
    expect(limiter.consume("partner", "CONTRACT_A").allowed).toBe(true);

    // 3rd request for contract A is rate limited
    const resA = limiter.consume("partner", "CONTRACT_A");
    expect(resA.allowed).toBe(false);
    expect(resA.limitType).toBe("contract");
    expect(resA.resetAfterMs).toBeGreaterThan(0);

    // Contract B is NOT affected by contract A's exhaustion (contract isolation)
    expect(limiter.consume("partner", "CONTRACT_B").allowed).toBe(true);

    // Different API key is also NOT affected on contract A (API key isolation)
    limiter.registerKey("partner2", "standard");
    expect(limiter.consume("partner2", "CONTRACT_A").allowed).toBe(true);
  });

  test("contract-specific limits do not disable overall key budget", () => {
    const limiter = new RateLimiter(() => 1000, 100);
    limiter.registerKey("free-key", "free"); // 60 req/min overall

    // Consume 60 requests across multiple contracts
    for (let i = 0; i < 60; i++) {
      expect(limiter.consume("free-key", `CONTRACT_${i % 10}`).allowed).toBe(true);
    }

    // 61st request is blocked by key-level limit even though individual contract limit is 100
    const res = limiter.consume("free-key", "CONTRACT_99");
    expect(res.allowed).toBe(false);
    expect(res.limitType).toBe("key");
  });

  test("contract identifier normalization handles missing, whitespace, and case", () => {
    expect(normalizeContractId("")).toBe("__unknown_contract__");
    expect(normalizeContractId(null as any)).toBe("__unknown_contract__");
    expect(normalizeContractId("  c_abc123  ")).toBe("C_ABC123");
    expect(normalizeContractId("invalid$#@")).toBe("__invalid_contract__");

    const limiter = new RateLimiter();
    limiter.registerKey("k", "free");
    // Missing contract ID falls back predictably
    expect(limiter.consume("k", "").allowed).toBe(true);
    limiter.reset("k", "c_abc123");
  });

  // ── Issue #932 Authentication & Security Tests ─────────────────────────────
  test("authenticator validates API keys and derives identity", () => {
    const auth = new Authenticator();
    auth.registerKey("sec-key-123", "standard", "partner-service");
    auth.registerKey("admin-key-999", "enterprise", "super-admin", true);

    const caller1 = auth.authenticate({ "x-api-key": "sec-key-123" });
    expect(caller1).not.toBeNull();
    expect(caller1?.identity).toBe("partner-service");
    expect(caller1?.tier).toBe("standard");
    expect(caller1?.isAdmin).toBe(false);

    const callerAdmin = auth.authenticate({ authorization: "ApiKey admin-key-999" });
    expect(callerAdmin?.isAdmin).toBe(true);

    // Unregistered key rejected
    expect(auth.authenticate({ "x-api-key": "unknown-key" })).toBeNull();
  });

  test("authenticator validates JWTs with signature, expiry, issuer, and audience", () => {
    const secret = "test-secret-1234567890-test-secret-1234567890";
    const auth = new Authenticator({
      jwtSecret: secret,
      jwtIssuer: "kora-auth",
      jwtAudience: "kora-gateway",
      clockSkewSeconds: 5,
    });

    const nowSec = 1700000000;
    const nowMs = nowSec * 1000;

    // 1. Valid token
    const validToken = makeJwt(
      {
        sub: "user-42",
        iss: "kora-auth",
        aud: "kora-gateway",
        exp: nowSec + 3600,
        nbf: nowSec - 60,
        role: "admin",
      },
      secret,
    );
    const caller = auth.authenticate({ authorization: `Bearer ${validToken}` }, nowMs);
    expect(caller).not.toBeNull();
    expect(caller?.identity).toBe("user-42");
    expect(caller?.isAdmin).toBe(true);

    // 2. Tampered signature rejected
    const tampered = validToken.slice(0, -4) + "XXXX";
    expect(auth.authenticate({ authorization: `Bearer ${tampered}` }, nowMs)).toBeNull();

    // 3. Expired token rejected
    const expiredToken = makeJwt(
      { sub: "user-42", exp: nowSec - 100 },
      secret,
    );
    expect(auth.authenticate({ authorization: `Bearer ${expiredToken}` }, nowMs)).toBeNull();

    // 4. Issuer mismatch rejected
    const badIssuerToken = makeJwt(
      { sub: "user-42", iss: "wrong-issuer", exp: nowSec + 3600 },
      secret,
    );
    expect(auth.authenticate({ authorization: `Bearer ${badIssuerToken}` }, nowMs)).toBeNull();

    // 5. Audience mismatch rejected
    const badAudToken = makeJwt(
      { sub: "user-42", iss: "kora-auth", aud: "other-aud", exp: nowSec + 3600 },
      secret,
    );
    expect(auth.authenticate({ authorization: `Bearer ${badAudToken}` }, nowMs)).toBeNull();

    // 6. Merely present or malformed token rejected
    expect(auth.authenticate({ authorization: "Bearer not.a.valid.jwt" }, nowMs)).toBeNull();
    expect(auth.authenticate({ authorization: "Bearer " }, nowMs)).toBeNull();
  });

  test("secret redaction utility protects credentials in logs and errors", () => {
    expect(redactSecret("")).toBe("");
    expect(redactSecret("short")).toBe("[REDACTED]");
    expect(redactSecret("my-super-secret-api-key-12345")).toBe("my-s...2345[REDACTED]");

    const headers = {
      Authorization: "Bearer eyJhbGciOi...",
      "X-Api-Key": "secret-1234",
      "Content-Type": "application/json",
    };
    const redacted = redactHeaders(headers);
    expect(redacted["Authorization"]).toBe("[REDACTED]");
    expect(redacted["X-Api-Key"]).toBe("[REDACTED]");
    expect(redacted["Content-Type"]).toBe("application/json");
  });

  // ── Issue #932 HTTP Gateway Integration Tests ──────────────────────────────
  test("HTTP GET /health is unauthenticated and returns 200", async () => {
    const gw = new ApiGateway(seedStore());
    const res = await simulateRequest(gw, "GET", "/health");
    expect(res.statusCode).toBe(200);
    expect(res.body).toEqual({ status: "ok" });
  });

  test("HTTP mutating route requires authentication (401 on missing/invalid)", async () => {
    const gw = new ApiGateway(seedStore());

    // Missing auth -> 401
    const resNoAuth = await simulateRequest(gw, "POST", "/contracts/CAAAAAA/invoke", {}, { action: "mint" });
    expect(resNoAuth.statusCode).toBe(401);
    expect(resNoAuth.body).toEqual({ error: "Unauthorized" });

    // Invalid auth -> 401
    const resBadAuth = await simulateRequest(
      gw,
      "POST",
      "/contracts/CAAAAAA/invoke",
      { Authorization: "Bearer invalid.jwt.token" },
      { action: "mint" },
    );
    expect(resBadAuth.statusCode).toBe(401);
    expect(resBadAuth.body).toEqual({ error: "Unauthorized" });
  });

  test("HTTP mutating route enforces contract-scoped rate limit with 429 and Retry-After", async () => {
    const secret = "test-secret-key-12345678901234567890";
    const auth = new Authenticator({ jwtSecret: secret });
    const limiter = new RateLimiter(() => 1000, 2); // 2 requests per contract
    const gw = new ApiGateway(seedStore(), limiter, auth);

    const token = makeJwt({ sub: "caller-1", exp: 2000000000 }, secret);
    const headers = { Authorization: `Bearer ${token}` };

    // Request 1 & 2 succeed
    const res1 = await simulateRequest(gw, "POST", "/contracts/CPOL123/invoke", headers, { arg: 1 });
    expect(res1.statusCode).toBe(200);
    expect(res1.body.ok).toBe(true);

    const res2 = await simulateRequest(gw, "POST", "/contracts/CPOL123/invoke", headers, { arg: 2 });
    expect(res2.statusCode).toBe(200);

    // Request 3 hits contract rate limit -> 429 with Retry-After header
    const res3 = await simulateRequest(gw, "POST", "/contracts/CPOL123/invoke", headers, { arg: 3 });
    expect(res3.statusCode).toBe(429);
    expect(res3.headers["Retry-After"]).toBeDefined();
    expect(res3.body.error).toBe("Too Many Requests");
    expect(res3.body.retryAfter).toBeGreaterThan(0);

    // Call to a different contract still passes
    const resOtherContract = await simulateRequest(gw, "POST", "/contracts/CPOL456/invoke", headers, { arg: 1 });
    expect(resOtherContract.statusCode).toBe(200);
  });

  test("HTTP admin routes reject authenticated non-admin caller with 403", async () => {
    const secret = "test-secret-key-12345678901234567890";
    const auth = new Authenticator({ jwtSecret: secret });
    const gw = new ApiGateway(seedStore(), new RateLimiter(), auth);

    // Non-admin token
    const nonAdminToken = makeJwt({ sub: "user-1", role: "user", exp: 2000000000 }, secret);
    const resForbidden = await simulateRequest(
      gw,
      "POST",
      "/admin/config",
      { Authorization: `Bearer ${nonAdminToken}` },
      { param: "val" },
    );
    expect(resForbidden.statusCode).toBe(403);
    expect(resForbidden.body).toEqual({ error: "Forbidden" });

    // Admin token
    const adminToken = makeJwt({ sub: "admin-1", role: "admin", exp: 2000000000 }, secret);
    const resAdmin = await simulateRequest(
      gw,
      "POST",
      "/admin/config",
      { Authorization: `Bearer ${adminToken}` },
      { param: "val" },
    );
    expect(resAdmin.statusCode).toBe(200);
  });

  test("HTTP read endpoints query indexed store correctly via HTTP", async () => {
    const gw = new ApiGateway(seedStore());

    const resInvoices = await simulateRequest(gw, "GET", "/invoices?status=Listed&limit=2");
    expect(resInvoices.statusCode).toBe(200);
    expect(resInvoices.body.data).toHaveLength(2);

    const resListings = await simulateRequest(gw, "GET", "/listings?token=USDC");
    expect(resListings.statusCode).toBe(200);
    expect(resListings.body.data).toHaveLength(2);

    const resPositions = await simulateRequest(gw, "GET", "/positions?investor=INV_1");
    expect(resPositions.statusCode).toBe(200);
    expect(resPositions.body.data).toHaveLength(1);

    const resRisk = await simulateRequest(gw, "GET", "/risk-scores?verifier=V1");
    expect(resRisk.statusCode).toBe(200);
    expect(resRisk.body.data).toHaveLength(2);

    const res404 = await simulateRequest(gw, "GET", "/non-existent");
    expect(res404.statusCode).toBe(404);
  });

  test("edge cases in JWT decoding, algorithms, and timestamps", () => {
    const secret = "my-secret-key-0000000000000000000000";
    const auth = new Authenticator({ jwtSecret: secret });

    // Malformed JSON in header
    const badHeaderJwt = `invalidHeader.${Buffer.from("{}").toString("base64url")}.sig`;
    expect(auth.authenticate({ authorization: `Bearer ${badHeaderJwt}` })).toBeNull();

    // Unsupported algorithm (e.g. HS512 or none)
    const noneHeader = Buffer.from(JSON.stringify({ alg: "none" })).toString("base64url");
    const payload = Buffer.from(JSON.stringify({ sub: "u" })).toString("base64url");
    expect(auth.authenticate({ authorization: `Bearer ${noneHeader}.${payload}.sig` })).toBeNull();

    // Malformed JSON in payload
    const validHeader = Buffer.from(JSON.stringify({ alg: "HS256" })).toString("base64url");
    const sig = createHmacSha256Signature(`${validHeader}.notJson`, secret);
    expect(auth.authenticate({ authorization: `Bearer ${validHeader}.notJson.${sig}` })).toBeNull();

    // nbf in the future
    const futureToken = makeJwt({ sub: "u", nbf: 2500000000 }, secret);
    expect(auth.authenticate({ authorization: `Bearer ${futureToken}` }, 1000)).toBeNull();

    // Array audience matching
    const audAuth = new Authenticator({ jwtSecret: secret, jwtAudience: "expected-aud" });
    const arrayAudToken = makeJwt({ sub: "u", aud: ["aud1", "expected-aud"], exp: 2000000000 }, secret);
    expect(audAuth.authenticate({ authorization: `Bearer ${arrayAudToken}` })).not.toBeNull();
  });

  test("authenticator loads from environment variables", () => {
    process.env.GATEWAY_API_KEYS = "env-free:free,env-std:standard,env-ent:enterprise";
    process.env.GATEWAY_ADMIN_KEYS = "env-admin";
    const envAuth = new Authenticator();
    expect(envAuth.authenticate({ "x-api-key": "env-free" })?.tier).toBe("free");
    expect(envAuth.authenticate({ "x-api-key": "env-std" })?.tier).toBe("standard");
    expect(envAuth.authenticate({ "x-api-key": "env-ent" })?.tier).toBe("enterprise");
    expect(envAuth.authenticate({ "x-api-key": "env-admin" })?.isAdmin).toBe(true);
    delete process.env.GATEWAY_API_KEYS;
    delete process.env.GATEWAY_ADMIN_KEYS;
  });

  test("RateLimiter custom contract limit and full key reset", () => {
    const limiter = new RateLimiter();
    limiter.registerContractLimit("CONTRACT_SPECIAL", { maxRequests: 5, windowMs: 10_000 });
    limiter.registerKey("k", "free");
    for (let i = 0; i < 5; i++) {
      expect(limiter.consume("k", "CONTRACT_SPECIAL").allowed).toBe(true);
    }
    expect(limiter.consume("k", "CONTRACT_SPECIAL").allowed).toBe(false);

    // Resetting key clears contract-scoped buckets too
    limiter.reset("k");
    expect(limiter.consume("k", "CONTRACT_SPECIAL").allowed).toBe(true);
  });

  test("HTTP mutating route handles malformed JSON body", async () => {
    const secret = "test-secret-key-12345678901234567890";
    const auth = new Authenticator({ jwtSecret: secret });
    const gw = new ApiGateway(seedStore(), new RateLimiter(), auth);

    const token = makeJwt({ sub: "user-1", exp: 2000000000 }, secret);
    const rawReq = new (require("stream").Readable)();
    rawReq.method = "POST";
    rawReq.url = "/contracts/C1/invoke";
    rawReq.headers = { authorization: `Bearer ${token}` };
    rawReq.push("invalid-json{");
    rawReq.push(null);

    let status = 200;
    let data = "";
    const rawRes: any = {
      writeHead: (s: number) => { status = s; },
      end: (c: string) => { data = c; },
    };

    await gw.handleHttpRequest(rawReq as any, rawRes as any);
    expect(status).toBe(400);
    expect(JSON.parse(data)).toEqual({ error: "Invalid JSON body" });
  });

  test("HTTP read endpoints handle rate limiting", async () => {
    const limiter = new RateLimiter(() => 1000);
    limiter.registerKey("read-free", "free");
    const gw = new ApiGateway(seedStore(), limiter);

    for (let i = 0; i < 60; i++) {
      const res = await simulateRequest(gw, "GET", "/invoices", { "x-api-key": "read-free" });
      expect(res.statusCode).toBe(200);
    }
    const limitedRes = await simulateRequest(gw, "GET", "/invoices", { "x-api-key": "read-free" });
    expect(limitedRes.statusCode).toBe(429);
    expect(limitedRes.headers["Retry-After"]).toBeDefined();
  });

  test("createHttpServer starts an http.Server", (done) => {
    const gw = new ApiGateway(seedStore());
    const server = gw.createHttpServer();
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address() as any;
      http.get(`http://127.0.0.1:${addr.port}/health`, (res) => {
        expect(res.statusCode).toBe(200);
        server.close(() => done());
      });
    });
  });

  test("additional edge cases for branch coverage", async () => {
    // 1. Whitespace contractId
    expect(normalizeContractId("    ")).toBe("__unknown_contract__");

    // 2. createMemoryStore fully populated
    const customStore = createMemoryStore({
      invoices: [],
      listings: [],
      positions: [],
      riskScores: [],
      lastIndexedLedger: 99,
    });
    expect(customStore.lastIndexedLedger).toBe(99);

    // 3. Authenticator with validApiKeys Map
    const keyMap = new Map([["explicit-key", { tier: "enterprise" as const, identity: "explicit-identity" }]]);
    const authWithMap = new Authenticator({ validApiKeys: keyMap });
    const authRes = authWithMap.authenticate({ "x-api-key": "explicit-key" });
    expect(authRes).not.toBeNull();
    expect(authRes?.identity).toBe("explicit-identity");

    // 4. SortBy with equal values
    const equalItemsStore = createMemoryStore({
      invoices: [
        { id: "inv-1", sme: "G1", debtorHash: "h1", amount: "100", currency: "USDC", dueDate: 100, ipfsCid: "c1", riskScore: 10, riskTier: "A", status: "Listed", createdAt: 1 },
        { id: "inv-2", sme: "G2", debtorHash: "h2", amount: "100", currency: "USDC", dueDate: 100, ipfsCid: "c2", riskScore: 10, riskTier: "A", status: "Listed", createdAt: 1 },
      ],
    });
    const gwEqual = new ApiGateway(equalItemsStore);
    const sorted = gwEqual.queryInvoices({}, {}, { apiKey: "anon", sortBy: "amount", sortDir: "asc" });
    expect(sorted.data.length).toBe(2);

    // 5. Empty body mutation request
    const auth = new Authenticator({ validApiKeys: new Map([["mut-key", { tier: "standard" as const, identity: "user-mut" }]]) });
    const gw = new ApiGateway(seedStore(), new RateLimiter(), auth);
    const rawReq = new (require("stream").Readable)();
    rawReq.method = "POST";
    rawReq.url = "/contracts/C1/invoke";
    rawReq.headers = { "x-api-key": "mut-key" };
    rawReq.push(""); // empty string
    rawReq.push(null);

    let status = 0;
    let data = "";
    const rawRes: any = {
      writeHead: (s: number) => { status = s; },
      end: (c: string) => { data = c; },
    };
    await gw.handleHttpRequest(rawReq as any, rawRes as any);
    expect(status).toBe(200);
    expect(JSON.parse(data).data).toEqual({});

    // 6. createHttpServer error handling (500)
    const faultyGw = new ApiGateway(seedStore());
    jest.spyOn(faultyGw, "handleHttpRequest").mockRejectedValueOnce(new Error("unexpected crash"));
    const server = faultyGw.createHttpServer();
    await new Promise<void>((resolve) => {
      server.listen(0, "127.0.0.1", () => {
        const addr = server.address() as any;
        http.get(`http://127.0.0.1:${addr.port}/crash`, (res) => {
          expect(res.statusCode).toBe(500);
          server.close(() => resolve());
        });
      });
    });

    // 7. createMemoryStore default empty store
    const emptyStore = createMemoryStore();
    expect(emptyStore.invoices).toEqual([]);
    expect(emptyStore.listings).toEqual([]);
    expect(emptyStore.positions).toEqual([]);
    expect(emptyStore.riskScores).toEqual([]);
    expect(emptyStore.lastIndexedLedger).toBe(0);

    // 8. Authenticator registerKey default identity
    const regAuth = new Authenticator();
    regAuth.registerKey("default-key-12345", "free");
    const regRes = regAuth.authenticate({ "x-api-key": "default-key-12345" });
    expect(regRes?.identity).toBe("apikey:default-");

    // 9. JWT payloads with userId, apiKey, and no sub
    const secretKey = "test-secret-key-12345678901234567890";
    const jwtAuth = new Authenticator({ jwtSecret: secretKey });
    const tokenUserId = makeJwt({ userId: "user-via-id", exp: 2000000000 }, secretKey);
    const tokenApiKey = makeJwt({ apiKey: "key-via-jwt-123", exp: 2000000000 }, secretKey);
    const tokenNone = makeJwt({ exp: 2000000000 }, secretKey);

    expect(jwtAuth.authenticate({ authorization: `Bearer ${tokenUserId}` })?.identity).toBe("user-via-id");
    expect(jwtAuth.authenticate({ authorization: `Bearer ${tokenApiKey}` })?.identity).toBe("apikey:key-via-");
    expect(jwtAuth.authenticate({ authorization: `Bearer ${tokenNone}` })?.identity).toBe("jwt_user");

    // 10. Unexpected errors in handleHttpRequest rethrow
    const throwingLimiter = new RateLimiter();
    (throwingLimiter as any).consume = jest.fn().mockImplementationOnce(() => {
      throw new Error("unexpected limiter failure");
    });
    const gwThrowing = new ApiGateway(seedStore(), throwingLimiter, new Authenticator({ validApiKeys: keyMap }));
    await expect(
      simulateRequest(gwThrowing, "POST", "/contracts/C1/invoke", { "x-api-key": "explicit-key" }, {}),
    ).rejects.toThrow("unexpected limiter failure");

    (throwingLimiter as any).consume = jest.fn().mockImplementationOnce(() => {
      throw new Error("unexpected query limiter failure");
    });
    await expect(
      simulateRequest(gwThrowing, "GET", "/invoices", { "x-api-key": "explicit-key" }),
    ).rejects.toThrow("unexpected query limiter failure");

    // 11. Query endpoints with full search params
    const gwFull = new ApiGateway(seedStore());
    const resInv = await simulateRequest(
      gwFull,
      "GET",
      "/invoices?status=Listed&sme=SME_A&currency=USDC&sortBy=amount&sortDir=desc&limit=5&cursor=MA==",
      { authorization: "Bearer anon-token" },
    );
    expect(resInv.statusCode).toBe(200);

    const resList = await simulateRequest(
      gwFull,
      "GET",
      "/listings?seller=SME_A&token=USDC",
    );
    expect(resList.statusCode).toBe(200);

    const resPos = await simulateRequest(
      gwFull,
      "GET",
      "/positions?investor=INV_1&invoiceId=1",
    );
    expect(resPos.statusCode).toBe(200);

    const resRisk = await simulateRequest(
      gwFull,
      "GET",
      "/risk-scores?verifier=V1",
    );
    expect(resRisk.statusCode).toBe(200);

    // 12. Default method arguments on ApiGateway
    const optsDefault = { apiKey: "test-default" };
    expect(gwFull.queryInvoices(undefined, undefined, optsDefault).data).toBeDefined();
    expect(gwFull.queryListings(undefined, undefined, optsDefault).data).toBeDefined();
    expect(gwFull.queryPositions(undefined, undefined, optsDefault).data).toBeDefined();
    expect(gwFull.queryRiskScores(undefined, undefined, optsDefault).data).toBeDefined();

    // 13. matches filter with gte, lte, in
    const resFilterAdvanced = gwFull.queryInvoices(
      { riskScore: { gte: 15, lte: 75, in: [20, 70] } },
      {},
      optsDefault,
    );
    expect(resFilterAdvanced.data.length).toBeGreaterThan(0);
  });
});

