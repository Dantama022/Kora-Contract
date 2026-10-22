import { writeFileSync, existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

// Read contract spec names from extracted fixtures or default contracts
const fixturesDir = "contracts/xtask/tests/fixtures";
let contractNames: string[] = ["invoice_nft", "marketplace", "price_oracle", "risk_registry"];

if (existsSync(fixturesDir)) {
  const fixtureFiles = readdirSync(fixturesDir).filter((f) => f.endsWith(".spec.json"));
  if (fixtureFiles.length > 0) {
    const discovered = fixtureFiles.map((f) => f.replace(/\.spec\.json$/, ""));
    contractNames = Array.from(new Set([...contractNames, ...discovered])).sort();
  }
}

const unionTypes = contractNames.map((n) => `"${n}"`).join(" | ");
let generated = `// Auto-generated from extracted contract specs (Issue #937)\n`;
generated += `export type ContractSpecName = ${unionTypes};\n\n`;

// Include loaded spec map if invoice_nft fixture is available
if (existsSync(join(fixturesDir, "invoice_nft.spec.json"))) {
  const invoiceSpec = JSON.parse(readFileSync(join(fixturesDir, "invoice_nft.spec.json"), "utf8"));
  generated += `export const INVOICE_NFT_SPEC = ${JSON.stringify(invoiceSpec, null, 2)} as const;\n`;
}

writeFileSync("sdk/src/generated-types.ts", generated);
console.log("Generated sdk/src/generated-types.ts with contract specs:", contractNames.join(", "));
