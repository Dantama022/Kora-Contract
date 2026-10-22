/**
 * API Gateway — Query & Mutation Gateway Layer
 *
 * Stateless query layer over an `IndexedStore` with robust authentication and
 * contract-scoped rate limiting:
 * - Public read-only endpoints: /health (unauthenticated), /invoices, /listings, /positions, /risk-scores.
 * - Mutating endpoints: require validated API key or JWT authentication; reject unauthenticated calls with 401.
 * - Contract-scoped & per-key rate limits: breaches return 429 with `Retry-After`.
 * - No credentials or internal secrets leak into logs or responses.
 *
 * Issue #753, Issue #932
 */

import * as http from "http";
import { URL } from "url";
import pino from "pino";
import {
  FieldFilter,
  InvoiceFilter,
  InvoiceRecord,
  ListingFilter,
  ListingRecord,
  Page,
  PageArgs,
  PositionFilter,
  PositionRecord,
  RiskScoreFilter,
  RiskScoreRecord,
  SortDir,
} from "./types";
import { IndexedStore } from "./store";
import { RateLimiter, normalizeContractId } from "./rateLimiter";
import { Authenticator, redactHeaders } from "./auth";

const logger = pino({ name: "api-gateway" });

export class RateLimitedError extends Error {
  public retryAfterSeconds: number;
  public limitType?: "key" | "contract";

  constructor(public resetAfterMs: number, limitType?: "key" | "contract") {
    super(`RATE_LIMITED: retry after ${resetAfterMs}ms`);
    this.retryAfterSeconds = Math.max(1, Math.ceil(resetAfterMs / 1000));
    this.limitType = limitType;
  }
}

function matches<T>(value: T, filter?: FieldFilter<T>): boolean {
  if (!filter) return true;
  if (filter.eq !== undefined && value !== filter.eq) return false;
  if (filter.in !== undefined && !filter.in.includes(value)) return false;
  if (filter.gte !== undefined && (value as unknown as number) < (filter.gte as unknown as number)) return false;
  if (filter.lte !== undefined && (value as unknown as number) > (filter.lte as unknown as number)) return false;
  return true;
}

function decodeCursor(cursor?: string): number {
  if (!cursor) return 0;
  const n = Number(Buffer.from(cursor, "base64").toString("utf8"));
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : 0;
}

function encodeCursor(offset: number): string {
  return Buffer.from(String(offset)).toString("base64");
}

function paginate<T>(items: T[], args: PageArgs, lastIndexedLedger: number): Page<T> {
  const limit = Math.max(1, Math.min(100, args.limit ?? 20));
  const offset = decodeCursor(args.cursor);
  const data = items.slice(offset, offset + limit);
  const next = offset + data.length < items.length ? encodeCursor(offset + data.length) : null;
  return { data, nextCursor: next, lastIndexedLedger };
}

export interface QueryOptions {
  apiKey: string;
  sortBy?: string;
  sortDir?: SortDir;
  contractId?: string;
}

function sortBy<T>(items: T[], key: string | undefined, dir: SortDir | undefined): T[] {
  if (!key) return items;
  const d = dir === "desc" ? -1 : 1;
  return [...items].sort((a, b) => {
    const av = (a as Record<string, unknown>)[key] as unknown as number | string;
    const bv = (b as Record<string, unknown>)[key] as unknown as number | string;
    if (av < bv) return -1 * d;
    if (av > bv) return 1 * d;
    return 0;
  });
}

function sendJson(res: http.ServerResponse, status: number, body: unknown, headers: Record<string, string> = {}): void {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(payload),
    ...headers,
  });
  res.end(payload);
}

export class ApiGateway {
  constructor(
    private store: IndexedStore,
    private limiter: RateLimiter = new RateLimiter(),
    private authenticator: Authenticator = new Authenticator(),
  ) {}

  checkRateLimit(apiKey: string, contractId?: string): void {
    const res = this.limiter.consume(apiKey, contractId);
    if (!res.allowed) {
      throw new RateLimitedError(res.resetAfterMs, res.limitType);
    }
  }

  queryInvoices(filter: InvoiceFilter = {}, page: PageArgs = {}, opts: QueryOptions): Page<InvoiceRecord> {
    this.checkRateLimit(opts.apiKey, opts.contractId);
    const items = sortBy(
      this.store.invoices.filter(
        (i) =>
          matches(i.sme, filter.sme) &&
          matches(i.status, filter.status) &&
          matches(i.riskTier, filter.riskTier) &&
          matches(i.riskScore, filter.riskScore) &&
          matches(i.currency, filter.currency),
      ),
      opts.sortBy,
      opts.sortDir,
    );
    return paginate(items, page, this.store.lastIndexedLedger);
  }

  queryListings(filter: ListingFilter = {}, page: PageArgs = {}, opts: QueryOptions): Page<ListingRecord> {
    this.checkRateLimit(opts.apiKey, opts.contractId);
    const items = sortBy(
      this.store.listings.filter(
        (l) =>
          matches(l.seller, filter.seller) &&
          matches(l.token, filter.token) &&
          (filter.isActive === undefined || l.isActive === filter.isActive),
      ),
      opts.sortBy,
      opts.sortDir,
    );
    return paginate(items, page, this.store.lastIndexedLedger);
  }

  queryPositions(filter: PositionFilter = {}, page: PageArgs = {}, opts: QueryOptions): Page<PositionRecord> {
    this.checkRateLimit(opts.apiKey, opts.contractId);
    const items = sortBy(
      this.store.positions.filter(
        (p) => matches(p.investor, filter.investor) && matches(p.invoiceId, filter.invoiceId),
      ),
      opts.sortBy,
      opts.sortDir,
    );
    return paginate(items, page, this.store.lastIndexedLedger);
  }

  queryRiskScores(filter: RiskScoreFilter = {}, page: PageArgs = {}, opts: QueryOptions): Page<RiskScoreRecord> {
    this.checkRateLimit(opts.apiKey, opts.contractId);
    const items = sortBy(
      this.store.riskScores.filter(
        (r) =>
          (filter.kind === undefined || r.kind === filter.kind) &&
          matches(r.verifier, filter.verifier) &&
          matches(r.score, filter.score),
      ),
      opts.sortBy,
      opts.sortDir,
    );
    return paginate(items, page, this.store.lastIndexedLedger);
  }

  /**
   * HTTP request dispatcher handling routing, authentication, and contract rate limiting.
   */
  async handleHttpRequest(req: http.IncomingMessage, res: http.ServerResponse): Promise<void> {
    const rawUrl = req.url ?? "/";
    const parsedUrl = new URL(rawUrl, "http://127.0.0.1");
    const pathname = parsedUrl.pathname;
    const method = (req.method ?? "GET").toUpperCase();

    // 1. Health check: unauthenticated public route
    if (method === "GET" && pathname === "/health") {
      return sendJson(res, 200, { status: "ok" });
    }

    // 2. Mutating routes (require authentication)
    const isMutation = method === "POST" || method === "PUT" || method === "DELETE" || method === "PATCH";
    if (isMutation) {
      // Authenticate caller strictly using credentials
      const caller = this.authenticator.authenticate(req.headers);
      if (!caller) {
        logger.warn({ headers: redactHeaders(req.headers), path: pathname }, "Unauthorized mutation attempt rejected");
        return sendJson(res, 401, { error: "Unauthorized" });
      }

      // Check admin permission if admin route
      if (pathname.startsWith("/admin/") && !caller.isAdmin) {
        logger.warn({ identity: caller.identity, path: pathname }, "Forbidden admin action attempt");
        return sendJson(res, 403, { error: "Forbidden" });
      }

      // Extract contract identifier if present, e.g. /contracts/:contractId/invoke
      const contractMatch = pathname.match(/^\/contracts\/([^/]+)/);
      const rawContractId = contractMatch ? contractMatch[1] : undefined;
      const contractId = rawContractId ? normalizeContractId(rawContractId) : undefined;

      // Rate limit check for mutation
      try {
        this.checkRateLimit(caller.identity, contractId);
      } catch (err) {
        if (err instanceof RateLimitedError) {
          logger.info({ identity: caller.identity, contractId, retryAfter: err.retryAfterSeconds }, "Mutation rate limit exceeded");
          return sendJson(
            res,
            429,
            { error: "Too Many Requests", retryAfter: err.retryAfterSeconds },
            { "Retry-After": String(err.retryAfterSeconds) },
          );
        }
        throw err;
      }

      // Read mutation body
      let body: unknown = {};
      try {
        body = await this.readJsonBody(req);
      } catch {
        return sendJson(res, 400, { error: "Invalid JSON body" });
      }

      logger.info({ identity: caller.identity, contractId, path: pathname }, "Mutation request processed successfully");
      return sendJson(res, 200, {
        ok: true,
        caller: caller.identity,
        contractId: contractId ?? null,
        data: body,
      });
    }

    // 3. Public query routes (read-only)
    const apiKey =
      (req.headers["x-api-key"] as string | undefined) ??
      (req.headers["authorization"] ? String(req.headers["authorization"]).replace(/^Bearer\s+/, "") : "anon");

    const limitParam = parsedUrl.searchParams.get("limit");
    const cursor = parsedUrl.searchParams.get("cursor") ?? undefined;
    const sortByParam = parsedUrl.searchParams.get("sortBy") ?? undefined;
    const sortDirParam = (parsedUrl.searchParams.get("sortDir") as SortDir) ?? undefined;
    const contractParam = parsedUrl.searchParams.get("contractId") ?? undefined;
    const pageArgs: PageArgs = {
      limit: limitParam ? parseInt(limitParam, 10) : undefined,
      cursor,
    };
    const queryOpts: QueryOptions = {
      apiKey,
      sortBy: sortByParam,
      sortDir: sortDirParam,
      contractId: contractParam,
    };

    try {
      if (method === "GET" && pathname === "/invoices") {
        const status = parsedUrl.searchParams.get("status");
        const sme = parsedUrl.searchParams.get("sme");
        const currency = parsedUrl.searchParams.get("currency");
        const filter: InvoiceFilter = {
          status: status ? { eq: status } : undefined,
          sme: sme ? { eq: sme } : undefined,
          currency: currency ? { eq: currency } : undefined,
        };
        const page = this.queryInvoices(filter, pageArgs, queryOpts);
        return sendJson(res, 200, page);
      }

      if (method === "GET" && pathname === "/listings") {
        const seller = parsedUrl.searchParams.get("seller");
        const token = parsedUrl.searchParams.get("token");
        const filter: ListingFilter = {
          seller: seller ? { eq: seller } : undefined,
          token: token ? { eq: token } : undefined,
        };
        const page = this.queryListings(filter, pageArgs, queryOpts);
        return sendJson(res, 200, page);
      }

      if (method === "GET" && pathname === "/positions") {
        const investor = parsedUrl.searchParams.get("investor");
        const invoiceId = parsedUrl.searchParams.get("invoiceId");
        const filter: PositionFilter = {
          investor: investor ? { eq: investor } : undefined,
          invoiceId: invoiceId ? { eq: invoiceId } : undefined,
        };
        const page = this.queryPositions(filter, pageArgs, queryOpts);
        return sendJson(res, 200, page);
      }

      if (method === "GET" && pathname === "/risk-scores") {
        const verifier = parsedUrl.searchParams.get("verifier");
        const filter: RiskScoreFilter = {
          verifier: verifier ? { eq: verifier } : undefined,
        };
        const page = this.queryRiskScores(filter, pageArgs, queryOpts);
        return sendJson(res, 200, page);
      }
    } catch (err) {
      if (err instanceof RateLimitedError) {
        return sendJson(
          res,
          429,
          { error: "Too Many Requests", retryAfter: err.retryAfterSeconds },
          { "Retry-After": String(err.retryAfterSeconds) },
        );
      }
      throw err;
    }

    return sendJson(res, 404, { error: "Not Found" });
  }

  createHttpServer(): http.Server {
    return http.createServer((req, res) => {
      this.handleHttpRequest(req, res).catch((err) => {
        logger.error({ err: err instanceof Error ? err.message : "internal_error" }, "Unhandled error in API Gateway");
        sendJson(res, 500, { error: "Internal Server Error" });
      });
    });
  }

  private readJsonBody(req: http.IncomingMessage): Promise<unknown> {
    return new Promise((resolve, reject) => {
      let data = "";
      req.setEncoding("utf8");
      req.on("data", (chunk) => {
        data += chunk;
      });
      req.on("end", () => {
        if (!data.trim()) {
          resolve({});
          return;
        }
        try {
          resolve(JSON.parse(data));
        } catch (e) {
          reject(e);
        }
      });
      req.on("error", reject);
    });
  }
}
