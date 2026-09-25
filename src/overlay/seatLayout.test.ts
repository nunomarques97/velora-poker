// Unit tests for the pure seat-layout engine. Run with `npm run test:layout`
// (tsc to .test-build/, then node:test; no extra dependency).
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  MEDIUM_TABLE_SIZE,
  MIN_FONT_PX,
  MIN_TABLE_SIZE,
  TABLE_CENTRE,
  chipMetrics,
  chipRect,
  chipScale,
  defaultSeatAnchors,
  holeCardsRect,
  layoutSeats,
  protectedZones,
  rectsOverlap,
  seatKeys,
  seatPlateRect,
  type ChipMetrics,
  type ChipPlacement,
  type Rect,
  type SeatFrame,
} from "./seatLayout.js";

const SIZES: ReadonlyArray<readonly [number, number]> = [
  [483, 359],
  [640, 455],
  [800, 570],
];
const TABLES = [6, 9] as const;

function rectOf(p: ChipPlacement, chip: ChipMetrics, w: number, h: number): Rect {
  return chipRect({ x: p.x, y: p.y }, chip, w, h);
}

function inWindow(r: Rect): boolean {
  const eps = 1e-9;
  return r.x >= -eps && r.y >= -eps && r.x + r.w <= 1 + eps && r.y + r.h <= 1 + eps;
}

function label(n: number, frame: SeatFrame, w: number, h: number): string {
  return `${n}-max ${frame} at ${w}x${h}`;
}

describe("chip scale", () => {
  it("keeps text at 10px or more at the minimum table size", () => {
    const m = chipMetrics(MIN_TABLE_SIZE.width, MIN_TABLE_SIZE.height);
    assert.ok(m.fontPx >= MIN_FONT_PX, `font ${m.fontPx}px`);
  });

  it("never drops below 10px text for any window, even smaller than the minimum", () => {
    for (const [w, h] of [[200, 150], [483, 359], [0, 0], [Number.NaN, 400]] as const) {
      assert.ok(chipMetrics(w, h).fontPx >= MIN_FONT_PX, `${w}x${h}`);
    }
  });

  it("does not grow beyond the medium table size", () => {
    const medium = chipMetrics(MEDIUM_TABLE_SIZE.width, MEDIUM_TABLE_SIZE.height);
    const big = chipMetrics(1600, 1140);
    assert.equal(chipScale(1600, 1140), 1);
    assert.equal(big.widthPx, medium.widthPx);
    assert.equal(big.heightPx, medium.heightPx);
    assert.equal(big.fontPx, medium.fontPx);
  });

  it("is monotonic between the minimum and medium size", () => {
    let last = 0;
    for (let w = 483; w <= 800; w += 20) {
      const s = chipScale(w, (w * 570) / 800);
      assert.ok(s >= last - 1e-12, `scale fell at width ${w}`);
      last = s;
    }
  });

  it("sizes the badge smaller than the compact chip", () => {
    const compact = chipMetrics(800, 570, "compact");
    const badge = chipMetrics(800, 570, "badge");
    assert.ok(badge.widthPx < compact.widthPx);
    assert.equal(badge.fontPx, compact.fontPx);
  });
});

describe("default anchors and zones", () => {
  it("has one anchor per seat key for every size 2..10 in both frames, inside the window", () => {
    for (let n = 2; n <= 10; n++) {
      for (const frame of ["hero", "absolute"] as const) {
        const anchors = defaultSeatAnchors(n, frame);
        assert.deepEqual(
          anchors.map((a) => a.seatKey),
          seatKeys(n, frame),
        );
        assert.deepEqual(
          [...anchors.map((a) => a.slot)].sort((a, b) => a - b),
          Array.from({ length: n }, (_, i) => i),
          `${n}-max ${frame} slots`,
        );
        for (const a of anchors) {
          assert.ok(a.x > 0 && a.x < 1 && a.y > 0 && a.y < 1, `${n}-max ${frame} seat ${a.seatKey}`);
        }
      }
    }
  });

  it("puts the hero at the bottom centre in the hero frame", () => {
    for (let n = 2; n <= 10; n++) {
      const hero = defaultSeatAnchors(n, "hero")[0];
      assert.equal(hero.seatKey, 0);
      assert.ok(Math.abs(hero.x - 0.5) < 1e-9, `${n}-max hero x`);
      for (const other of defaultSeatAnchors(n, "hero").slice(1)) {
        assert.ok(other.y <= hero.y + 1e-9, `${n}-max seat ${other.seatKey} below the hero`);
      }
    }
  });

  it("orders the absolute frame as a clockwise ring by seat number", () => {
    for (const n of TABLES) {
      const slots = defaultSeatAnchors(n, "absolute").map((a) => a.slot);
      for (let i = 1; i < n; i++) {
        assert.equal(slots[i], (slots[0] + i) % n, `${n}-max seat ${i + 1}`);
      }
    }
  });

  it("protects the board, every bet box, the hero's hole cards and the action buttons", () => {
    for (const n of TABLES) {
      const zones = protectedZones(n, "hero");
      const count = (kind: string) => zones.filter((z) => z.kind === kind).length;
      assert.equal(count("board"), 1);
      assert.equal(count("bet"), n);
      assert.equal(count("holeCards"), 1);
      assert.equal(count("actions"), 1);
      assert.equal(zones.find((z) => z.kind === "holeCards")?.seatKey, 0);
    }
    assert.equal(protectedZones(6, "absolute").filter((z) => z.kind === "holeCards").length, 0);
    assert.equal(protectedZones(6, "absolute", 4).find((z) => z.kind === "holeCards")?.seatKey, 4);
  });
});

describe("default layout", () => {
  for (const n of TABLES) {
    for (const frame of ["hero", "absolute"] as const) {
      for (const [w, h] of SIZES) {
        it(`${label(n, frame, w, h)}: no overlaps, no protected zone, in bounds`, () => {
          const layout = layoutSeats({ maxPlayers: n, frame, windowW: w, windowH: h, heroSeatKey: frame === "absolute" ? 1 : undefined });
          assert.equal(layout.positions.length, n);
          const rects = layout.positions.map((p) => rectOf(p, layout.chip, w, h));
          for (let i = 0; i < rects.length; i++) {
            assert.ok(inWindow(rects[i]), `seat ${layout.positions[i].seatKey} out of the window`);
            for (let j = i + 1; j < rects.length; j++) {
              assert.ok(
                !rectsOverlap(rects[i], rects[j]),
                `seats ${layout.positions[i].seatKey} and ${layout.positions[j].seatKey} overlap`,
              );
            }
            for (const zone of protectedZones(n, frame, frame === "absolute" ? 1 : undefined)) {
              assert.ok(
                !rectsOverlap(rects[i], zone.rect),
                `seat ${layout.positions[i].seatKey} covers ${zone.kind}${zone.seatKey ?? ""}`,
              );
            }
          }
        });
      }
    }
  }

  for (const n of TABLES) {
    for (const [w, h] of SIZES) {
      it(`${label(n, "hero", w, h)}: every chip sits on the outer side of its plate`, () => {
        const layout = layoutSeats({ maxPlayers: n, frame: "hero", windowW: w, windowH: h });
        const anchors = new Map(defaultSeatAnchors(n, "hero").map((a) => [a.seatKey, a]));
        for (const p of layout.positions) {
          const a = anchors.get(p.seatKey as number)!;
          const outX = (a.x - TABLE_CENTRE.x) * w;
          const outY = (a.y - TABLE_CENTRE.y) * h;
          const dot = (p.x - a.x) * w * outX + (p.y - a.y) * h * outY;
          assert.ok(dot > 0, `seat ${p.seatKey} faces the table centre`);
        }
      });
    }
  }

  for (const n of TABLES) {
    for (const frame of ["hero", "absolute"] as const) {
      for (const [w, h] of SIZES) {
        it(`${label(n, frame, w, h)}: no chip covers any seat's hole cards or plate`, () => {
          const layout = layoutSeats({ maxPlayers: n, frame, windowW: w, windowH: h, heroSeatKey: frame === "absolute" ? 1 : undefined });
          const anchors = defaultSeatAnchors(n, frame);
          for (const p of layout.positions) {
            const r = rectOf(p, layout.chip, w, h);
            for (const a of anchors) {
              assert.ok(!rectsOverlap(r, holeCardsRect(a)), `seat ${p.seatKey} covers seat ${a.seatKey}'s hole cards`);
              assert.ok(!rectsOverlap(r, seatPlateRect(a)), `seat ${p.seatKey} covers seat ${a.seatKey}'s plate`);
            }
          }
        });
      }
    }
  }

  it("keeps every size from 2 to 10 collision-free at the minimum table size", () => {
    for (let n = 2; n <= 10; n++) {
      const { width: w, height: h } = MIN_TABLE_SIZE;
      const layout = layoutSeats({ maxPlayers: n, frame: "hero", windowW: w, windowH: h });
      const rects = layout.positions.map((p) => rectOf(p, layout.chip, w, h));
      rects.forEach((r, i) => {
        assert.ok(inWindow(r), `${n}-max seat ${i}`);
        rects.slice(i + 1).forEach((o) => assert.ok(!rectsOverlap(r, o), `${n}-max seat ${i}`));
        protectedZones(n, "hero").forEach((z) => assert.ok(!rectsOverlap(r, z.rect), `${n}-max seat ${i} ${z.kind}`));
      });
    }
  });

  it("returns only the requested seats, in the requested order, at the same place as a full layout", () => {
    const full = layoutSeats({ maxPlayers: 9, frame: "hero", windowW: 483, windowH: 359 });
    const some = layoutSeats({ maxPlayers: 9, frame: "hero", windowW: 483, windowH: 359, seats: [5, 2, 42] });
    assert.deepEqual(
      some.positions.map((p) => p.seatKey),
      [5, 2],
    );
    assert.deepEqual(some.positions[0], full.positions[5]);
    assert.deepEqual(some.positions[1], full.positions[2]);
  });

  it("is deterministic", () => {
    const input = {
      maxPlayers: 9,
      frame: "hero" as const,
      windowW: 483,
      windowH: 359,
      overrides: [{ seatKey: 3, x: 0.2, y: 0.4 }],
      spareCount: 2,
    };
    const a = layoutSeats(input);
    const b = layoutSeats({ ...input, overrides: [...input.overrides] });
    assert.deepEqual(a, b);
  });
});

describe("overrides", () => {
  it("keeps an in-bounds override exactly", () => {
    const layout = layoutSeats({
      maxPlayers: 6,
      frame: "hero",
      windowW: 640,
      windowH: 455,
      overrides: [{ seatKey: 2, x: 0.4123, y: 0.2345 }],
    });
    const p = layout.positions.find((q) => q.seatKey === 2)!;
    assert.equal(p.x, 0.4123);
    assert.equal(p.y, 0.2345);
    assert.equal(p.overridden, true);
    assert.equal(layout.positions.filter((q) => q.overridden).length, 1);
  });

  it("keeps an override exactly even over a protected zone", () => {
    const layout = layoutSeats({
      maxPlayers: 9,
      frame: "absolute",
      windowW: 800,
      windowH: 570,
      overrides: [{ seatKey: 4, x: 0.5, y: 0.45 }],
    });
    const p = layout.positions.find((q) => q.seatKey === 4)!;
    assert.deepEqual([p.x, p.y], [0.5, 0.45]);
  });

  it("clamps an out-of-range override so the whole chip stays in the window", () => {
    for (const [x, y] of [[1.7, -0.4], [-3, 2], [Number.NaN, 1], [1, 1], [0, 0]]) {
      const layout = layoutSeats({
        maxPlayers: 6,
        frame: "hero",
        windowW: 483,
        windowH: 359,
        overrides: [{ seatKey: 1, x, y }],
      });
      const p = layout.positions.find((q) => q.seatKey === 1)!;
      assert.ok(inWindow(rectOf(p, layout.chip, 483, 359)), `override ${x},${y}`);
      assert.equal(p.overridden, true);
    }
  });

  it("treats overrides as fixed obstacles for the defaults", () => {
    for (const n of TABLES) {
      for (const [w, h] of SIZES) {
        const plain = layoutSeats({ maxPlayers: n, frame: "hero", windowW: w, windowH: h });
        // Drop seat 1's chip exactly where seat 2's default would go.
        const target = plain.positions[2];
        const layout = layoutSeats({
          maxPlayers: n,
          frame: "hero",
          windowW: w,
          windowH: h,
          overrides: [{ seatKey: 1, x: target.x, y: target.y }],
        });
        const moved = layout.positions.find((p) => p.seatKey === 1)!;
        assert.deepEqual([moved.x, moved.y], [target.x, target.y]);
        const fixed = rectOf(moved, layout.chip, w, h);
        for (const p of layout.positions) {
          if (p.seatKey === 1) continue;
          assert.ok(!rectsOverlap(fixed, rectOf(p, layout.chip, w, h)), `${label(n, "hero", w, h)} seat ${p.seatKey}`);
        }
      }
    }
  });

  it("ignores overrides for seats the table does not have", () => {
    const plain = layoutSeats({ maxPlayers: 6, frame: "hero", windowW: 800, windowH: 570 });
    const layout = layoutSeats({
      maxPlayers: 6,
      frame: "hero",
      windowW: 800,
      windowH: 570,
      overrides: [{ seatKey: 6, x: 0.1, y: 0.1 }, { seatKey: -1, x: 0.2, y: 0.2 }],
    });
    assert.deepEqual(layout.positions, plain.positions);
  });
});

describe("spare slots", () => {
  it("places spares without overlapping chips, protected zones or the window edge", () => {
    for (const n of TABLES) {
      const [w, h] = [483, 359];
      const layout = layoutSeats({ maxPlayers: n, frame: "hero", windowW: w, windowH: h, spareCount: 2 });
      assert.equal(layout.spares.length, 2);
      const all = [...layout.positions, ...layout.spares].map((p) => rectOf(p, layout.chip, w, h));
      all.forEach((r, i) => {
        assert.ok(inWindow(r));
        all.slice(i + 1).forEach((o) => assert.ok(!rectsOverlap(r, o), `${n}-max chip ${i}`));
      });
      for (const s of layout.spares) {
        assert.equal(s.seatKey, null);
        for (const z of protectedZones(n, "hero")) {
          assert.ok(!rectsOverlap(rectOf(s, layout.chip, w, h), z.rect), `${n}-max spare over ${z.kind}`);
        }
      }
    }
  });
});
