import { RiskPipeline } from "../src/scorer";
import { FormulaRegistry, FORMULA_V1 } from "../src/formulas";
import { PaymentHistoryFactor } from "../src/factors";

describe("risk-pipeline (issue #755)", () => {
  test("factor breakdown is auditable and weights renormalize", () => {
    const pipeline = new RiskPipeline();
    const result = pipeline.score(
      {
        sme: {
          repayments: [{ paidOnTime: true }, { paidOnTime: false }, { paidOnTime: true }],
          defaults: 0,
          totalInvoices: 12,
          invoicesLast90d: 3,
        },
        debtors: [{ debtorHash: "ab".repeat(32), exposureShareBps: 10_000, avgDebtorScore: 30 }],
      },
      "v1",
    );
    expect(result.formulaVersion).toBe("v1");
    expect(result.score).toBeGreaterThanOrEqual(0);
    expect(result.score).toBeLessThanOrEqual(100);
    // v1 ignores macro_trade: applied weight must be 0 and flagged missing.
    const macro = result.breakdown.find((b) => b.factorId === "macro_trade");
    expect(macro?.appliedWeight).toBe(0);
    // Applied weights of usable factors sum to ~1.
    const appliedSum = result.breakdown.reduce((a, b) => a + b.appliedWeight, 0);
    expect(appliedSum).toBeCloseTo(1, 5);
    expect(result.breakdown.length).toBe(4);
  });

  test("sparse data degrades confidence instead of failing", () => {
    const pipeline = new RiskPipeline();
    const full = pipeline.score(
      {
        sme: {
          repayments: Array.from({ length: 20 }, () => ({ paidOnTime: true })),
          totalInvoices: 20,
          invoicesLast90d: 3,
          defaults: 0,
        },
        debtors: [
          { debtorHash: "a", exposureShareBps: 5000 },
          { debtorHash: "b", exposureShareBps: 5000 },
        ],
      },
      "v1",
    );
    const sparse = pipeline.score({}, "v1");
    expect(sparse.warnings).toContain("SPARSE_DATA");
    expect(sparse.confidence).toBeLessThan(full.confidence);
    expect(sparse.score).toBe(50); // neutral fallback
    expect(sparse.warnings).toContain("LOW_CONFIDENCE_NOT_RECOMMENDED");
  });

  test("versioned formulas are backward compatible and immutable", () => {
    const pipeline = new RiskPipeline();
    const v1 = pipeline.score(
      {
        sme: { repayments: [{ paidOnTime: true }], totalInvoices: 5, invoicesLast90d: 1, defaults: 0 },
        debtors: [{ debtorHash: "x", exposureShareBps: 10_000 }],
        macro: { stressIndex: 90 },
      },
      "v1",
    );
    const v2 = pipeline.score(
      {
        sme: { repayments: [{ paidOnTime: true }], totalInvoices: 5, invoicesLast90d: 1, defaults: 0 },
        debtors: [{ debtorHash: "x", exposureShareBps: 10_000 }],
        macro: { stressIndex: 90 },
      },
      "v2",
    );
    // v2 sees macro stress, v1 ignores it → v2 must score riskier here.
    expect(v2.score).toBeGreaterThan(v1.score);
    expect(pipeline.listFormulaVersions()).toEqual(["v1", "v2"]);

    const registry = new FormulaRegistry([FORMULA_V1]);
    expect(() =>
      registry.register({ ...FORMULA_V1, weights: { ...FORMULA_V1.weights, payment_history: 0.9, invoice_frequency: 0.05, debtor_concentration: 0.05, macro_trade: 0 } }),
    ).toThrow(/immutable/);
    expect(() => registry.get("v99")).toThrow(/Unknown formula/);
  });

  test("payment history penalizes defaults and rewards clean records", () => {
    const f = new PaymentHistoryFactor();
    expect(f.describe()).toContain("SME repayment discipline");
    const clean = f.evaluate({ sme: { repayments: [{ paidOnTime: true }, { paidOnTime: true }], defaults: 0 } });
    const bad = f.evaluate({ sme: { repayments: [{ paidOnTime: false }], defaults: 2 } });
    expect(clean.score).toBeLessThan(bad.score);
    const missing = f.evaluate({});
    expect(missing.missing).toBe(true);
    expect(missing.confidence).toBe(0);
  });

  test("additional factor and formula coverage", () => {
    const { InvoiceFrequencyFactor, DebtorConcentrationFactor, MacroTradeFactor } = require("../src/factors");
    const invFactor = new InvoiceFrequencyFactor();
    expect(invFactor.describe()).toContain("cadence");
    const dormant = invFactor.evaluate({ sme: { totalInvoices: 0, invoicesLast90d: 0 } });
    expect(dormant.score).toBe(75);

    const debtorFactor = new DebtorConcentrationFactor();
    expect(debtorFactor.describe()).toContain("HHI");
    const emptyDebtors = debtorFactor.evaluate({ debtors: [] });
    expect(emptyDebtors.missing).toBe(true);

    const macroFactor = new MacroTradeFactor();
    expect(macroFactor.describe()).toContain("Systematic");
    const emptyMacro = macroFactor.evaluate({});
    expect(emptyMacro.missing).toBe(true);

    const registry = new FormulaRegistry();
    expect(registry.latest().version).toBe("v2");
    expect(() =>
      registry.register({
        version: "v_invalid",
        changelog: "test",
        weights: { payment_history: 0.2, invoice_frequency: 0.2, debtor_concentration: 0.2, macro_trade: 0.2 },
      })
    ).toThrow(/weights must sum to 1/);

    const pipeline = new RiskPipeline();
    // Use default latest version
    const defaultRes = pipeline.score({ sme: { totalInvoices: 5, invoicesLast90d: 2 } });
    expect(defaultRes.formulaVersion).toBe("v2");

    // Test macro factor partial inputs (stress only, fxVol only)
    const macroStressOnly = macroFactor.evaluate({ macro: { stressIndex: 40 } });
    expect(macroStressOnly.confidence).toBe(0.5);
    const macroFxOnly = macroFactor.evaluate({ macro: { fxVolatilityBps: 200 } });
    expect(macroFxOnly.confidence).toBe(0.5);
    const macroBoth = macroFactor.evaluate({ macro: { stressIndex: 40, fxVolatilityBps: 200 } });
    expect(macroBoth.confidence).toBe(0.9);

    // Test invoice frequency with small total (< 5)
    const lowTotal = invFactor.evaluate({ sme: { totalInvoices: 2, invoicesLast90d: 1 } });
    expect(lowTotal.score).toBeGreaterThan(0);

    // Test payment history with 0 repayments but > 0 defaults
    const defaultsOnly = (new PaymentHistoryFactor()).evaluate({ sme: { repayments: [], defaults: 1 } });
    expect(defaultsOnly.score).toBe(8);

    // Test debtor factor with multiple debtors having known and unknown scores
    const mixedDebtors = debtorFactor.evaluate({
      debtors: [
        { debtorHash: "d1", exposureShareBps: 4000, avgDebtorScore: 20 },
        { debtorHash: "d2", exposureShareBps: 6000 },
      ],
    });
    expect(mixedDebtors.score).toBeGreaterThan(0);

    // Test formula validation for negative weight
    expect(() =>
      registry.register({
        version: "v_neg",
        changelog: "test",
        weights: { payment_history: 1.2, invoice_frequency: -0.2, debtor_concentration: 0, macro_trade: 0 },
      })
    ).toThrow(/bad weight/);
  });
});
