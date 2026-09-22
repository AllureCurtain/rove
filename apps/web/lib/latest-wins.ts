/**
 * Serialize async UI work so only the newest request may write state, ported from
 * PI-Desktop's `lib/latest-wins.ts`.
 *
 * An effect-owned request can be dropped with a cleanup flag. A request started
 * from an event handler has no cleanup: ask for provider A's model list, ask for
 * B's before A answers, and A's slow response still lands — under B. `begin`
 * issues a token per request and only a token that is still current may commit;
 * `invalidate` covers teardown, where no in-flight response may land at all.
 */
export class LatestWinsGate {
  private current = 0;

  /** Claim the newest token for a request that is about to start. */
  begin(): number {
    this.current += 1;
    return this.current;
  }

  /** True while `token` is still the newest request. */
  isCurrent(token: number): boolean {
    return token === this.current;
  }

  /** Drop every in-flight request, including the one that is current. */
  invalidate(): void {
    this.current += 1;
  }
}
