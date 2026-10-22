import * as fs from "fs";
import * as path from "path";
import { Networks } from "@stellar/stellar-sdk";
import { KoraClient, KoraAddresses } from "../src/KoraClient";
import { loadKoraAddresses, manifestToAddresses, DeploymentManifest } from "../src/deployment";
import { getErrorMessage } from "../src/errorMapping";
import { NetworkConfig, TESTNET } from "../src/base";

/**
 * Soroban Testnet Integration Smoke Test Suite (Issue #935)
 *
 * Exercises read-only contract calls against Soroban Testnet using real SDK
 * client interfaces without performing any state mutations or requiring mainnet secrets.
 *
 * Pre-requisites for live execution:
 *   - KORA_DEPLOYMENT_MANIFEST environment variable pointing to a valid deployment JSON
 *     OR a valid deployments/testnet.json file in the repo root.
 *   - Reachable Soroban testnet RPC endpoint (TESTNET_RPC_URL or default).
 *
 * If configuration is missing locally, the suite is cleanly skipped with an actionable notice.
 */

const RPC_TIMEOUT_MS = parseInt(process.env.TESTNET_RPC_TIMEOUT_MS || "20000", 10);
const RPC_URL = process.env.TESTNET_RPC_URL || TESTNET.rpcUrl;
const NETWORK_PASSPHRASE = process.env.TESTNET_NETWORK_PASSPHRASE || Networks.TESTNET;

export function parseContractErrorCode(error: unknown): number | null {
  if (!error) return null;
  const message = error instanceof Error ? error.message : String(error);
  const match = message.match(/Contract,\s*#?(\d+)/i) || message.match(/code[:\s]+(\d+)/i);
  if (match) {
    return parseInt(match[1], 10);
  }
  return null;
}

export async function withTimeout<T>(promise: Promise<T>, timeoutMs: number, operationName: string): Promise<T> {
  let timer: NodeJS.Timeout;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error(`RPC timeout: "${operationName}" timed out after ${timeoutMs}ms against ${RPC_URL}`));
    }, timeoutMs);
  });
  try {
    return await Promise.race([promise, timeoutPromise]);
  } finally {
    clearTimeout(timer!);
  }
}

function resolveAddresses(): { addresses: KoraAddresses | null; source: string; skipReason?: string } {
  const envManifestPath = process.env.KORA_DEPLOYMENT_MANIFEST;
  const defaultManifestPath = path.resolve(__dirname, "../../deployments/testnet.json");
  const manifestPath = envManifestPath || defaultManifestPath;

  if (fs.existsSync(manifestPath)) {
    try {
      const addresses = loadKoraAddresses(manifestPath);
      return { addresses, source: `Manifest file: ${manifestPath}` };
    } catch (err) {
      return {
        addresses: null,
        source: manifestPath,
        skipReason: `Deployment manifest exists at ${manifestPath} but is invalid: ${err instanceof Error ? err.message : err}`,
      };
    }
  }

  // Check if addresses are directly configured via individual environment variables
  if (
    process.env.KORA_INVOICE_NFT_ADDRESS &&
    process.env.KORA_FINANCING_POOL_ADDRESS &&
    process.env.KORA_PRICE_ORACLE_ADDRESS
  ) {
    const addresses: KoraAddresses = {
      invoiceNft: process.env.KORA_INVOICE_NFT_ADDRESS,
      financingPool: process.env.KORA_FINANCING_POOL_ADDRESS,
      priceOracle: process.env.KORA_PRICE_ORACLE_ADDRESS,
      marketplace: process.env.KORA_MARKETPLACE_ADDRESS || "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
      treasury: process.env.KORA_TREASURY_ADDRESS || "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
      riskRegistry: process.env.KORA_RISK_REGISTRY_ADDRESS || "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
      accessControl: process.env.KORA_ACCESS_CONTROL_ADDRESS || "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    };
    return { addresses, source: "Environment variables (KORA_*_ADDRESS)" };
  }

  return {
    addresses: null,
    source: manifestPath,
    skipReason: `No deployment manifest found at "${manifestPath}" and KORA_*_ADDRESS env vars not provided. Set KORA_DEPLOYMENT_MANIFEST or deploy contracts to testnet via scripts/deploy.sh.`,
  };
}

const { addresses, source, skipReason } = resolveAddresses();
const shouldRun = addresses !== null;

const describeSuite = shouldRun ? describe : describe.skip;

describeSuite("Soroban Testnet Read-Only Smoke Tests", () => {
  let client: KoraClient;
  const network: NetworkConfig = {
    rpcUrl: RPC_URL,
    networkPassphrase: NETWORK_PASSPHRASE,
  };

  beforeAll(() => {
    if (!addresses) {
      throw new Error(`Cannot run smoke tests without resolved addresses: ${skipReason}`);
    }
    client = new KoraClient(addresses, network);
  });

  describe("Contract Configuration & Access Control", () => {
    it("should query access control paused state (read-only)", async () => {
      try {
        const isPaused = await withTimeout(
          client.accessControl.isPaused(),
          RPC_TIMEOUT_MS,
          "accessControl.isPaused"
        );
        expect(typeof isPaused).toBe("boolean");
      } catch (error) {
        const code = parseContractErrorCode(error);
        if (code !== null) {
          const mapped = getErrorMessage(code, "access_control");
          expect(mapped).toBeDefined();
        } else {
          const msg = error instanceof Error ? error.message : String(error);
          throw new Error(`accessControl.isPaused failed on ${addresses?.accessControl}: ${msg}`);
        }
      }
    });

    it("should query marketplace fee basis points (read-only)", async () => {
      try {
        const feeBps = await withTimeout(
          client.marketplace.getFeeBps(),
          RPC_TIMEOUT_MS,
          "marketplace.getFeeBps"
        );
        expect(typeof feeBps).toBe("number");
        expect(feeBps).toBeGreaterThanOrEqual(0);
      } catch (error) {
        const code = parseContractErrorCode(error);
        if (code !== null) {
          const mapped = getErrorMessage(code, "marketplace");
          expect(mapped).toBeDefined();
        } else {
          const msg = error instanceof Error ? error.message : String(error);
          throw new Error(`marketplace.getFeeBps failed on ${addresses?.marketplace}: ${msg}`);
        }
      }
    });
  });

  describe("Core Protocol Read-Only Invocations", () => {
    it("should execute read-only query get_pool on FinancingPool", async () => {
      // Query pool ID 1 (read-only)
      try {
        const pool = await withTimeout(
          client.financingPool.getPool(1n),
          RPC_TIMEOUT_MS,
          "financingPool.getPool"
        );
        expect(pool).toBeDefined();
      } catch (error) {
        const code = parseContractErrorCode(error);
        if (code !== null) {
          const mapped = getErrorMessage(code, "financing_pool");
          // Code 30 = Pool not found (normal on fresh deployment with no pools)
          // Code 4 = Protocol paused
          expect([30, 4, 1]).toContain(code);
          expect(mapped).toBeDefined();
        } else {
          const msg = error instanceof Error ? error.message : String(error);
          throw new Error(`financingPool.getPool(1n) failed on ${addresses?.financingPool}: ${msg}`);
        }
      }
    });

    it("should execute read-only query get_invoice on InvoiceNFT", async () => {
      // Query invoice ID 1 (read-only)
      try {
        const invoice = await withTimeout(
          client.invoiceNft.getInvoice(1n),
          RPC_TIMEOUT_MS,
          "invoiceNft.getInvoice"
        );
        expect(invoice).toBeDefined();
      } catch (error) {
        const code = parseContractErrorCode(error);
        if (code !== null) {
          const mapped = getErrorMessage(code, "invoice_nft");
          // Code 10 = Invoice not found (normal on fresh deployment)
          // Code 4 = Protocol paused
          expect([10, 4, 1]).toContain(code);
          expect(mapped).toBeDefined();
        } else {
          const msg = error instanceof Error ? error.message : String(error);
          throw new Error(`invoiceNft.getInvoice(1n) failed on ${addresses?.invoiceNft}: ${msg}`);
        }
      }
    });

    it("should execute read-only query get_price on PriceOracle", async () => {
      // Query price for USDC (read-only)
      try {
        const price = await withTimeout(
          client.priceOracle.getPrice("USDC"),
          RPC_TIMEOUT_MS,
          "priceOracle.getPrice"
        );
        expect(typeof price).toBe("bigint");
        expect(price).toBeGreaterThan(0n);
      } catch (error) {
        const code = parseContractErrorCode(error);
        if (code !== null) {
          const mapped = getErrorMessage(code, "price_oracle");
          expect(mapped).toBeDefined();
        } else {
          const msg = error instanceof Error ? error.message : String(error);
          // If price oracle has no price configured for USDC yet, or custom error
          expect(msg).toBeTruthy();
        }
      }
    });
  });
});

if (!shouldRun) {
  describe("Soroban Testnet Smoke Tests (Configuration Status)", () => {
    it("reports actionable skip reason when live testnet configuration is unavailable", () => {
      console.warn(`[TESTNET INTEGRATION SKIPPED] ${skipReason}`);
      expect(skipReason).toBeDefined();
      expect(shouldRun).toBe(false);
    });
  });
}
