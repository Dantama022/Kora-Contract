# Fee Clawback & Exposure Release Policy

## Fee clawback on failed listings (#975)

- `marketplace::fund_invoice` tracks per-investor `FeeContribution(invoice, investor)`.
- `marketplace::claim_refund` claws back `fee_contributed` via `treasury::refund_fee`
  (caller must be the authorized marketplace; fails with
  `InsufficientPoolBalance` when the treasury balance cannot cover it, e.g.
  fees already spent via grants).
- The full fee is reversed, including any referrer-split portion (fees are
  collected 100% to the treasury; referrer accounting is informational).
- The `FeeContribution` record is cleared after clawback and `RefundClaimed`
  is set first (CEI), so repeated refunds cannot double-clawback.
- Events: treasury emits `FEE_REV (marketplace, invoice_id, investor, amount,
  token, timestamp)`; marketplace emits `FEE_REV` with the listing id as the
  clawback reason scope.

## Token exposure release (#974)

- `TokenExposureCurrent(token)` is incremented at listing time and released on
  full funding, cancel, and — per this change — proportionally on each
  `claim_refund` by the refunded gross (`net + fee`) using checked subtraction
  (`safe_sub`), which returns `ArithmeticOverflow` instead of panicking on
  underflow. After all investors refund, the cap returns to its pre-listing level.
