// Seeded pseudo-random generator (sfc32 seeded through cyrb128), so a
// session is a pure function of its seed: same seed, same hand histories.

function cyrb128(text) {
  let h1 = 1779033703, h2 = 3144134277, h3 = 1013904242, h4 = 2773480762;
  for (let i = 0; i < text.length; i++) {
    const k = text.charCodeAt(i);
    h1 = h2 ^ Math.imul(h1 ^ k, 597399067);
    h2 = h3 ^ Math.imul(h2 ^ k, 2869860233);
    h3 = h4 ^ Math.imul(h3 ^ k, 951274213);
    h4 = h1 ^ Math.imul(h4 ^ k, 2716044179);
  }
  h1 = Math.imul(h3 ^ (h1 >>> 18), 597399067);
  h2 = Math.imul(h4 ^ (h2 >>> 22), 2869860233);
  h3 = Math.imul(h1 ^ (h3 >>> 17), 951274213);
  h4 = Math.imul(h2 ^ (h4 >>> 19), 2716044179);
  h1 ^= h2 ^ h3 ^ h4;
  h2 ^= h1;
  h3 ^= h1;
  h4 ^= h1;
  return [h1 >>> 0, h2 >>> 0, h3 >>> 0, h4 >>> 0];
}

/** A 32-bit unsigned hash of `text` (stable across runs and platforms). */
export function hash32(text) {
  return cyrb128(String(text))[0];
}

export class Rng {
  constructor(seed) {
    [this.a, this.b, this.c, this.d] = cyrb128(String(seed));
    for (let i = 0; i < 12; i++) this.next();
  }

  /** Uniform in [0, 1). */
  next() {
    let { a, b, c, d } = this;
    const t = (((a + b) | 0) + d) | 0;
    d = (d + 1) | 0;
    a = b ^ (b >>> 9);
    b = (c + (c << 3)) | 0;
    c = (c << 21) | (c >>> 11);
    c = (c + t) | 0;
    Object.assign(this, { a, b, c, d });
    return (t >>> 0) / 4294967296;
  }

  /** Integer in [0, n). */
  int(n) {
    return Math.floor(this.next() * n);
  }

  /** True with probability `p`. */
  chance(p) {
    return this.next() < p;
  }

  pick(items) {
    return items[this.int(items.length)];
  }

  /** In-place Fisher-Yates shuffle; returns `items`. */
  shuffle(items) {
    for (let i = items.length - 1; i > 0; i--) {
      const j = this.int(i + 1);
      [items[i], items[j]] = [items[j], items[i]];
    }
    return items;
  }

  /** An independent child stream, stable for a given label. */
  fork(label) {
    return new Rng(`${this.a}:${this.b}:${this.c}:${this.d}:${label}`);
  }
}
