// PokerStars text conventions: money, timestamps, headers and file names.
// Amounts are integer cents everywhere in the simulator, so a pot always
// adds up exactly; they become text only here.

/** `$0.25`, `$10` (whole amounts drop `.00`, as PokerStars writes them). */
export function money(cents, symbol = "$") {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  const whole = Math.floor(abs / 100);
  const frac = abs % 100;
  return `${sign}${symbol}${frac === 0 ? whole : `${whole}.${String(frac).padStart(2, "0")}`}`;
}

const pad = (n) => String(n).padStart(2, "0");

/** `2026/09/12 21:04:11` (hours unpadded, like the client). */
function stamp(date) {
  return `${date.getFullYear()}/${pad(date.getMonth() + 1)}/${pad(date.getDate())} ${date.getHours()}:${pad(
    date.getMinutes(),
  )}:${pad(date.getSeconds())}`;
}

/** Header timestamp: local time with the ET time in brackets. */
export function headerTime(date) {
  const et = new Date(date.getTime() - 5 * 3600 * 1000);
  return `${stamp(date)} WET [${stamp(et)} ET]`;
}

/** `20260912`: the date part of a hand-history file name. */
export function fileDate(date) {
  return `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}`;
}

/**
 * The first header line. Cash: `PokerStars Hand #N:  Hold'em No Limit
 * ($0.25/$0.50 USD) - ...`; Zoom: `PokerStars Zoom Hand #N:  Hold'em No
 * Limit ($0.05/$0.10) - ...` (no currency code, as in the real client).
 */
export function cashHeader({ handId, zoom, sb, bb, date }) {
  const prefix = zoom ? "PokerStars Zoom Hand" : "PokerStars Hand";
  const currency = zoom ? "" : " USD";
  return `${prefix} #${handId}:  Hold'em No Limit (${money(sb)}/${money(bb)}${currency}) - ${headerTime(date)}`;
}

/** `HH20260912 Halley - $0.05-$0.10 - USD No Limit Hold'em.txt`. */
export function cashFileName({ date, tableName, sb, bb }) {
  return `HH${fileDate(date)} ${tableName} - ${money(sb)}-${money(bb)} - USD No Limit Hold'em.txt`;
}

const STARS = [
  "Aegle", "Dares", "Halley", "Alcor", "Bellatrix", "Canopus", "Deneb", "Electra", "Fomalhaut", "Gienah",
  "Hadar", "Izar", "Jabbah", "Kochab", "Lesath", "Mirach", "Nashira", "Okul", "Phact", "Rasalas",
];
const ROMAN = ["II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X", "XI"];

/** A PokerStars-like cash table name (`Aegle IV`), unique per index. */
export function tableName(index, offset = 0) {
  const i = index + offset;
  return `${STARS[i % STARS.length]} ${ROMAN[Math.floor(i / STARS.length) % ROMAN.length]}`;
}

/** A Zoom pool name (`Halley`): Zoom files are named after the pool. */
export function zoomPoolName(offset = 0) {
  return STARS[offset % STARS.length];
}

// ------------------------------------------------------------ tournaments

/** Tournament chips: a bare integer (`11262`), as the client writes them. */
export function chips(n) {
  return String(n);
}

/** `$13.50`, `$3.00`: buy-in parts and bounties always carry the cents. */
export function money2(cents, symbol = "$") {
  return `${symbol}${Math.floor(cents / 100)}.${String(cents % 100).padStart(2, "0")}`;
}

/** Roman level number of a tournament header (`6` → `VI`). */
export function roman(n) {
  const digits = [
    [1000, "M"], [900, "CM"], [500, "D"], [400, "CD"], [100, "C"], [90, "XC"],
    [50, "L"], [40, "XL"], [10, "X"], [9, "IX"], [5, "V"], [4, "IV"], [1, "I"],
  ];
  let out = "";
  for (const [value, text] of digits) {
    while (n >= value) {
      out += text;
      n -= value;
    }
  }
  return out;
}

/** `$5.00+$5.00+$1.00`: buy-in components in cents. */
export function buyInText(parts) {
  return parts.map((p) => money2(p)).join("+");
}

/**
 * `PokerStars Hand #N: Tournament #T, $5.00+$5.00+$1.00 USD Hold'em No
 * Limit - Level VI (75/150) - ...` (one space after the colon, unlike cash).
 */
export function tournamentHeader({ handId, tournamentId, buyIn, level, sb, bb, date }) {
  return `PokerStars Hand #${handId}: Tournament #${tournamentId}, ${buyInText(buyIn)} USD Hold'em No Limit - Level ${roman(
    level,
  )} (${chips(sb)}/${chips(bb)}) - ${headerTime(date)}`;
}

/** `HH20260912 T4100000001 No Limit Hold'em $5.00 + $5.00 + $1.00.txt`. */
export function tournamentFileName({ date, tournamentId, buyIn }) {
  return `HH${fileDate(date)} T${tournamentId} No Limit Hold'em ${buyIn.map((p) => money2(p)).join(" + ")}.txt`;
}

/** `1st`, `2nd`, `3rd`, `11th`: a finishing place. */
export function ordinal(n) {
  const tens = n % 100;
  if (tens >= 11 && tens <= 13) return `${n}th`;
  return `${n}${{ 1: "st", 2: "nd", 3: "rd" }[n % 10] ?? "th"}`;
}
