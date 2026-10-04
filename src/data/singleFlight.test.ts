// Unit tests for singleFlight (the hands-imported refresh coalescing). Run
// with `npm run test:layout`.
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { singleFlight } from "./singleFlight.js";

/** A refresh whose runs resolve only when the test says so. */
function controlled() {
  const pending: Array<() => void> = [];
  let calls = 0;
  const run = () => {
    calls += 1;
    return new Promise<void>((resolve) => pending.push(resolve));
  };
  return {
    run,
    calls: () => calls,
    finish: async () => {
      pending.shift()?.();
      // Let the .finally chain and a follow-up run start.
      for (let i = 0; i < 5; i++) await Promise.resolve();
    },
  };
}

const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

describe("singleFlight", () => {
  it("runs once per trigger when triggers do not overlap", async () => {
    const r = controlled();
    const trigger = singleFlight(r.run);
    trigger();
    await settle();
    assert.equal(r.calls(), 1);
    await r.finish();
    trigger();
    await settle();
    assert.equal(r.calls(), 2);
  });

  it("collapses a burst during a run into exactly one more run after it", async () => {
    const r = controlled();
    const trigger = singleFlight(r.run);
    trigger();
    await settle();
    for (let i = 0; i < 50; i++) trigger();
    await settle();
    assert.equal(r.calls(), 1, "never two runs at once");
    await r.finish();
    assert.equal(r.calls(), 2, "one trailing run reads the newest state");
    await r.finish();
    assert.equal(r.calls(), 2, "nothing left over");
  });

  it("keeps going after a failed run", async () => {
    let calls = 0;
    const trigger = singleFlight(() => {
      calls += 1;
      return Promise.reject(new Error("database busy"));
    });
    trigger();
    await settle();
    trigger();
    await settle();
    assert.equal(calls, 2);
  });

  it("does not hold back a refresh that returns nothing", async () => {
    let calls = 0;
    const trigger = singleFlight(() => {
      calls += 1;
    });
    trigger();
    await settle();
    trigger();
    await settle();
    assert.equal(calls, 2);
  });

  it("stops waiting for a run that never settles", async () => {
    let calls = 0;
    const trigger = singleFlight(() => {
      calls += 1;
      return calls === 1 ? new Promise<void>(() => undefined) : undefined;
    }, 20);
    trigger();
    await settle();
    trigger();
    await settle();
    assert.equal(calls, 1, "held back while the first run may still answer");
    await new Promise<void>((resolve) => setTimeout(resolve, 40));
    await settle();
    assert.equal(calls, 2, "the queued trigger runs once the first run is stale");
    trigger();
    await settle();
    assert.equal(calls, 3);
  });

  it("ignores the late settle of a stale run", async () => {
    const r = controlled();
    const trigger = singleFlight(r.run, 20);
    trigger();
    await settle();
    await new Promise<void>((resolve) => setTimeout(resolve, 40));
    trigger();
    await settle();
    assert.equal(r.calls(), 2);
    trigger();
    await r.finish(); // the stale first run settles late
    assert.equal(r.calls(), 2, "the second run is still in flight: no extra run");
    await r.finish();
    assert.equal(r.calls(), 3, "the trigger queued during it runs after it");
  });
});
