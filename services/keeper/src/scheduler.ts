/**
 * Keeper Service — Scheduler
 *
 * Orchestrates the tick loop:
 *   1. Promote expired pending jobs → ready
 *   2. Fetch all dispatchable jobs
 *   3. Dispatch each, recording outcome
 *   4. Move exhausted jobs to dead-letter
 *
 * Issue #765
 */

import { JobStore } from "./jobStore";
import { Dispatcher } from "./dispatcher";
import { KeeperConfig, Deadline } from "./types";
import pino from "pino";

const logger = pino({ name: "keeper:scheduler" });

export class Scheduler {
  private timer: ReturnType<typeof setInterval> | null = null;
  private running = false;

  constructor(
    private readonly store: JobStore,
    private readonly dispatcher: Dispatcher,
    private readonly config: KeeperConfig,
    private readonly scanListingTtl?: () => Promise<Deadline[]>
  ) {}

  /**
   * Enqueue a batch of deadlines sourced from the indexer.
   * Duplicate dedupKeys are silently ignored (idempotent).
   */
  enqueueDeadlines(deadlines: Deadline[]): void {
    for (const d of deadlines) {
      this.store.enqueue(d);
      logger.debug({ dedupKey: d.dedupKey }, "Deadline enqueued");
    }
  }

  /**
   * Start the periodic tick loop.
   */
  start(): void {
    if (this.running) return;
    this.running = true;
    logger.info(
      { intervalMs: this.config.pollIntervalMs },
      "Keeper scheduler started"
    );
    // Run once immediately, then on interval
    void this._tick();
    this.timer = setInterval(() => void this._tick(), this.config.pollIntervalMs);
  }

  /**
   * Gracefully stop the scheduler.
   */
  stop(): void {
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
    this.running = false;
    logger.info("Keeper scheduler stopped");
  }

  // ---------------------------------------------------------------------------
  // Tick
  // ---------------------------------------------------------------------------

  private async _tick(): Promise<void> {
    if (this.scanListingTtl) {
      try {
        const ttlJobs = await this.scanListingTtl();
        this.enqueueDeadlines(ttlJobs);
      } catch (error) {
        logger.error({ error }, "Listing TTL scan failed; existing jobs will continue");
      }
    }

    const nowTs = Math.floor(Date.now() / 1000);

    // Step 1: promote pending jobs whose deadline has elapsed
    const promoted = this.store.promoteExpired(nowTs);
    if (promoted > 0) {
      logger.info({ promoted }, "Jobs promoted to ready");
    }

    // Step 2: fetch dispatchable jobs
    const jobs = this.store.getDispatchable(
      nowTs,
      this.config.retryBackoffBaseMs,
      this.config.maxAttempts
    );

    if (jobs.length === 0) return;
    logger.info({ count: jobs.length }, "Dispatching jobs");

    // Step 3: dispatch sequentially (rate limiter inside Dispatcher)
    for (const job of jobs) {
      const result = await this.dispatcher.dispatch(job);
      this.store.recordAttempt(
        job.dedupKey,
        result.outcome,
        result.txHash,
        result.error
      );

      // Step 4: if exhausted, move to dead
      const updated = this.store.getByStatus("failed").find(
        (j) => j.dedupKey === job.dedupKey
      );
      if (updated && updated.attempts >= this.config.maxAttempts) {
        this.store.markDead(
          job.dedupKey,
          `Exhausted ${this.config.maxAttempts} attempts. Last error: ${updated.lastError}`
        );
        logger.warn({ dedupKey: job.dedupKey }, "Job moved to dead-letter queue");
      }
    }
  }
}
