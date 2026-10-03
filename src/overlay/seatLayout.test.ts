// Unit tests for the pure seat-layout engine. Run with `npm run test:layout`
// (tsc to .test-build/, then node:test; no extra dependency).
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  CHIP_SAMPLE_BASE_PX,
  CHIP_TAG_BASE_PX,
  HOVER_CARD_LINE_HEIGHT,
  MEDIUM_TABLE_SIZE,
  MIN_FONT_PX,
  MIN_TABLE_SIZE,
  TABLE_CENTRE,
  chipMetrics,
  chipRect,
  chipScale,
  defaultSeatAnchors,
  holeCardsRect,
  hoverCardHardZones,
  hoverCardMetrics,
  layoutSeats,
  placeHoverCard,
  protectedZones,
  rectsOverlap,
  seatKeys,
  seatPlateRect,
  type ChipMetrics,
  type ChipPlacement,
  type HoverCardInput,
  type HoverCardPlacement,
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

// ---------------------------------------------------------------------
// Tagged chips and the hover read card (strategic-analysis build).
// ---------------------------------------------------------------------

/** The two sizes the acceptance criteria name: PokerStars' minimum and default table. */
const CARD_SIZES: ReadonlyArray<readonly [number, number]> = [
  [MIN_TABLE_SIZE.width, MIN_TABLE_SIZE.height],
  [MEDIUM_TABLE_SIZE.width, MEDIUM_TABLE_SIZE.height],
];
const FRAMES = ["hero", "absolute"] as const;
const heroKeyFor = (frame: SeatFrame) => (frame === "absolute" ? 1 : undefined);

/**
 * Asserts the card's whole contract for one placement: inside the window,
 * over no protected zone (board/pot, bet chips, action buttons), over no
 * seat's hole cards (hero and opponents), not over its own chip, and at the
 * size it was given. The zones are listed here independently of
 * `hoverCardHardZones`, so a zone dropped there still fails.
 */
function assertCardSafe(input: HoverCardInput, placement: HoverCardPlacement, where: string) {
  const r = placement.rect;
  assert.ok(inWindow(r), `${where}: card leaves the window`);
  assert.ok(!rectsOverlap(r, input.chip), `${where}: card covers its own chip`);
  for (const zone of protectedZones(input.maxPlayers, input.frame, input.heroSeatKey)) {
    assert.ok(!rectsOverlap(r, zone.rect), `${where}: card covers ${zone.kind}${zone.seatKey ?? ""}`);
  }
  for (const anchor of defaultSeatAnchors(input.maxPlayers, input.frame)) {
    assert.ok(!rectsOverlap(r, holeCardsRect(anchor)), `${where}: card covers seat ${anchor.seatKey}'s hole cards`);
  }
  assert.ok(Math.abs(r.w * input.windowW - input.card.widthPx) < 1e-6, `${where}: card width changed`);
  assert.ok(Math.abs(r.h * input.windowH - input.card.heightPx) < 1e-6, `${where}: card height changed`);
}

/**
 * Card heights the tests place: two typical reads, two reads at the clamp,
 * and one read at the clamp (the name line and four lines, border, padding
 * and the 2px under the name, no rule).
 */
function cardHeights(w: number, h: number) {
  const m = hoverCardMetrics(w, h);
  const line = m.fontPx * HOVER_CARD_LINE_HEIGHT;
  return { m, typical: m.typicalHeightPx, max: m.maxHeightPx, oneRead: Math.ceil(5 * line + 12) };
}

describe("tagged chip", () => {
  it("adds the tag segment's and its sample's width, scaled like the rest of the chip", () => {
    for (const [w, h] of CARD_SIZES) {
      const plain = chipMetrics(w, h, "compact");
      const tagged = chipMetrics(w, h, "compact", true);
      const extra = (CHIP_TAG_BASE_PX + CHIP_SAMPLE_BASE_PX) * plain.scale;
      assert.ok(Math.abs(tagged.widthPx - plain.widthPx - extra) < 1e-9);
      assert.equal(tagged.fontPx, plain.fontPx);
      assert.equal(tagged.heightPx, plain.heightPx);
    }
    const min = chipMetrics(MIN_TABLE_SIZE.width, MIN_TABLE_SIZE.height, "compact", true);
    const medium = chipMetrics(MEDIUM_TABLE_SIZE.width, MEDIUM_TABLE_SIZE.height, "compact", true);
    assert.ok(min.fontPx >= MIN_FONT_PX, `tag text ${min.fontPx}px at 483x359`);
    assert.ok(medium.widthPx > min.widthPx && medium.fontPx > min.fontPx, "the tag scales up with the table");
  });

  for (const n of TABLES) {
    for (const frame of FRAMES) {
      for (const [w, h] of CARD_SIZES) {
        for (const model of ["compact", "badge"] as const) {
          it(`${label(n, frame, w, h)} ${model}: tag-width chips overlap no chip, no protected zone, stay in the window`, () => {
            const heroSeatKey = heroKeyFor(frame);
            const layout = layoutSeats({ maxPlayers: n, frame, windowW: w, windowH: h, model, tagged: true, heroSeatKey });
            assert.equal(layout.chip.widthPx, chipMetrics(w, h, model, true).widthPx);
            const rects = layout.positions.map((p) => rectOf(p, layout.chip, w, h));
            rects.forEach((r, i) => {
              const seat = layout.positions[i].seatKey;
              assert.ok(inWindow(r), `seat ${seat} out of the window`);
              rects.slice(i + 1).forEach((o, j) =>
                assert.ok(!rectsOverlap(r, o), `seats ${seat} and ${layout.positions[i + 1 + j].seatKey} overlap`),
              );
              for (const zone of protectedZones(n, frame, heroSeatKey)) {
                assert.ok(!rectsOverlap(r, zone.rect), `seat ${seat} covers ${zone.kind}${zone.seatKey ?? ""}`);
              }
            });
          });
        }
      }
    }
  }

  // The 9-max minimum-size crowding: a tag-width chip pushed off its own
  // spot used to land on its neighbour's name plate (seat 7 on seat 8's in
  // the hero frame, seat 3 on seat 4's in the absolute frame).
  for (const n of TABLES) {
    for (const frame of FRAMES) {
      for (const [w, h] of CARD_SIZES) {
        it(`${label(n, frame, w, h)}: a default chip touches no other seat's name plate, tagged or not`, () => {
          const heroSeatKey = heroKeyFor(frame);
          const anchors = defaultSeatAnchors(n, frame);
          for (const model of ["compact", "badge"] as const) {
            for (const tagged of [false, true]) {
              const layout = layoutSeats({ maxPlayers: n, frame, windowW: w, windowH: h, model, tagged, heroSeatKey });
              for (const p of layout.positions) {
                const r = rectOf(p, layout.chip, w, h);
                for (const a of anchors) {
                  if (a.seatKey === p.seatKey) continue;
                  assert.ok(
                    !rectsOverlap(r, seatPlateRect(a)),
                    `${model}${tagged ? " tagged" : ""}: seat ${p.seatKey}'s chip covers seat ${a.seatKey}'s plate`,
                  );
                }
              }
            }
          }
        });
      }
    }
  }
});

describe("hover read card", () => {
  it("keeps card text at 10px or more and scales it with the table", () => {
    const min = hoverCardMetrics(MIN_TABLE_SIZE.width, MIN_TABLE_SIZE.height);
    const medium = hoverCardMetrics(MEDIUM_TABLE_SIZE.width, MEDIUM_TABLE_SIZE.height);
    assert.ok(min.fontPx >= MIN_FONT_PX);
    assert.ok(medium.fontPx > min.fontPx && medium.widthPx > min.widthPx);
    assert.ok(min.widthPx < MIN_TABLE_SIZE.width / 2, "the card takes under half a minimum table's width");
    assert.ok(min.typicalHeightPx <= min.maxHeightPx);
  });

  for (const n of TABLES) {
    for (const frame of FRAMES) {
      for (const [w, h] of CARD_SIZES) {
        // At the medium size every chip gets a card (two reads, or the
        // overlay's one-read fallback). At the minimum size a 9-max table
        // has no room left for some seats once every seat's cards are hard
        // zones; those chips get no card (the drawer and the accessible
        // name still carry the reads), and any card that is placed is safe.
        const medium = w === MEDIUM_TABLE_SIZE.width;
        it(`${label(n, frame, w, h)}: every placed card is safe${medium ? ", and every default chip gets one" : ""}`, () => {
          const heroSeatKey = heroKeyFor(frame);
          const layout = layoutSeats({ maxPlayers: n, frame, windowW: w, windowH: h, tagged: true, heroSeatKey });
          const rects = layout.positions.map((p) => rectOf(p, layout.chip, w, h));
          const { m, typical, max, oneRead } = cardHeights(w, h);
          rects.forEach((chip, i) => {
            const seat = layout.positions[i].seatKey;
            const otherChips = rects.filter((_, j) => j !== i);
            const placements: Array<HoverCardPlacement | null> = [];
            for (const [what, height] of [["two typical reads", typical], ["one clamped read", oneRead], ["two clamped reads", max]] as const) {
              const input: HoverCardInput = {
                maxPlayers: n, frame, windowW: w, windowH: h, heroSeatKey, chip, otherChips,
                card: { widthPx: m.widthPx, heightPx: height },
              };
              const placement = placeHoverCard(input);
              placements.push(placement);
              if (!placement) continue;
              assertCardSafe(input, placement, `seat ${seat} ${what}`);
              if (placement.fit === "clear") {
                otherChips.forEach((o) => assert.ok(!rectsOverlap(placement.rect, o), `seat ${seat}: 'clear' card over a chip`));
              }
            }
            if (medium) assert.ok(placements[0] || placements[1], `seat ${seat}: no card at all at ${w}x${h}`);
          });
        });

        it(`${label(n, frame, w, h)}: chips dragged against every edge and corner get only safe cards${medium ? ", always one" : ""}`, () => {
          const heroSeatKey = heroKeyFor(frame);
          const chip = chipMetrics(w, h, "compact", true);
          const cw = chip.widthPx / w;
          const chh = chip.heightPx / h;
          const { m, typical, max, oneRead } = cardHeights(w, h);
          const steps = [0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1];
          const spots: Array<[number, number]> = [];
          for (const t of steps) spots.push([t, 0], [t, 1], [0, t], [1, t]);
          for (const [fx, fy] of spots) {
            const rect: Rect = { x: fx * (1 - cw), y: fy * (1 - chh), w: cw, h: chh };
            let placed = false;
            for (const height of [typical, oneRead, max]) {
              const input: HoverCardInput = {
                maxPlayers: n, frame, windowW: w, windowH: h, heroSeatKey, chip: rect,
                card: { widthPx: m.widthPx, heightPx: height },
              };
              const placement = placeHoverCard(input);
              if (!placement) continue;
              if (height !== max) placed = true;
              assertCardSafe(input, placement, `chip at edge ${fx},${fy}, card ${height}px tall`);
            }
            if (medium) assert.ok(placed, `chip at edge ${fx},${fy}: no card at all at ${w}x${h}`);
          }
        });
      }
    }
  }

  it("is safe for a chip anywhere, even one dragged over a protected zone, or returns null", () => {
    for (const n of TABLES) {
      for (const [w, h] of CARD_SIZES) {
        const chip = chipMetrics(w, h, "compact", true);
        const { m, max } = cardHeights(w, h);
        for (let fx = 0.05; fx < 1; fx += 0.15) {
          for (let fy = 0.05; fy < 1; fy += 0.15) {
            const input: HoverCardInput = {
              maxPlayers: n, frame: "hero", windowW: w, windowH: h,
              chip: chipRect({ x: fx, y: fy }, chip, w, h),
              card: { widthPx: m.widthPx, heightPx: max },
            };
            const placement = placeHoverCard(input);
            if (placement) assertCardSafe(input, placement, `${n}-max ${w}x${h} chip at ${fx.toFixed(2)},${fy.toFixed(2)}`);
          }
        }
      }
    }
  });

  it("makes every seat's hole cards a hard zone, the hero's and every opponent's", () => {
    for (const n of TABLES) {
      for (const frame of FRAMES) {
        const chip: Rect = { x: 0.1, y: 0.1, w: 0.1, h: 0.03 };
        const zones = hoverCardHardZones({
          maxPlayers: n, frame, windowW: 800, windowH: 570, heroSeatKey: heroKeyFor(frame), chip,
          card: { widthPx: 100, heightPx: 50 },
        });
        for (const a of defaultSeatAnchors(n, frame)) {
          assert.ok(zones.some((z) => JSON.stringify(z) === JSON.stringify(holeCardsRect(a))), `${n}-max ${frame} seat ${a.seatKey}`);
        }
        assert.deepEqual(zones[zones.length - 1], chip);
      }
    }
  });

  it("never puts the card over the top seat's cards at 800x570 (the D106 report)", () => {
    for (const n of TABLES) {
      const [w, h] = [MEDIUM_TABLE_SIZE.width, MEDIUM_TABLE_SIZE.height];
      const layout = layoutSeats({ maxPlayers: n, frame: "hero", windowW: w, windowH: h, tagged: true });
      const rects = layout.positions.map((p) => rectOf(p, layout.chip, w, h));
      const anchors = defaultSeatAnchors(n, "hero");
      const topY = Math.min(...anchors.map((a) => a.y));
      const top = anchors.filter((a) => Math.abs(a.y - topY) < 1e-9);
      const { m, typical } = cardHeights(w, h);
      rects.forEach((chip, i) => {
        const placement = placeHoverCard({
          maxPlayers: n, frame: "hero", windowW: w, windowH: h, chip,
          otherChips: rects.filter((_, j) => j !== i),
          card: { widthPx: m.widthPx, heightPx: typical },
        });
        if (!placement) return;
        for (const a of top) {
          assert.ok(!rectsOverlap(placement.rect, holeCardsRect(a)), `${n}-max seat ${layout.positions[i].seatKey} over top seat ${a.seatKey}`);
        }
      });
    }
  });

  it("returns null for a card that cannot fit, and is deterministic", () => {
    const base: HoverCardInput = {
      maxPlayers: 6, frame: "hero", windowW: 483, windowH: 359,
      chip: { x: 0.1, y: 0.1, w: 0.2, h: 0.05 },
      card: { widthPx: 600, heightPx: 80 },
    };
    assert.equal(placeHoverCard(base), null);
    assert.equal(placeHoverCard({ ...base, card: { widthPx: 0, heightPx: 80 } }), null);
    const ok: HoverCardInput = { ...base, card: { widthPx: 200, heightPx: 80 } };
    assert.deepEqual(placeHoverCard(ok), placeHoverCard({ ...ok, chip: { ...ok.chip } }));
  });

  it("puts the card right next to its chip when the felt around it is free", () => {
    // The left lower seat: open felt below the board. (The top seat has none
    // left once every seat's cards are hard zones; its card goes farther.)
    const [w, h] = [MEDIUM_TABLE_SIZE.width, MEDIUM_TABLE_SIZE.height];
    const layout = layoutSeats({ maxPlayers: 6, frame: "hero", windowW: w, windowH: h, tagged: true });
    const seat = layout.positions.find((p) => p.seatKey === 1)!;
    const chip = rectOf(seat, layout.chip, w, h);
    const { m, typical } = cardHeights(w, h);
    const placement = placeHoverCard({
      maxPlayers: 6, frame: "hero", windowW: w, windowH: h, chip,
      card: { widthPx: m.widthPx, heightPx: typical },
    });
    assert.ok(placement);
    const r = placement.rect;
    const gapX = Math.max(0, Math.max(r.x, chip.x) - Math.min(r.x + r.w, chip.x + chip.w)) * w;
    const gapY = Math.max(0, Math.max(r.y, chip.y) - Math.min(r.y + r.h, chip.y + chip.h)) * h;
    assert.ok(Math.hypot(gapX, gapY) <= 8, `card ${Math.hypot(gapX, gapY).toFixed(1)}px from its chip`);
  });
});
