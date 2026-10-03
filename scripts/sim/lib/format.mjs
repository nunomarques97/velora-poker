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
