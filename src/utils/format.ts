/** "2h 14m" / "45m" / "0m" from a whole-second duration. */
export function formatDuration(seconds: number): string {
  const totalMinutes = Math.round(seconds / 60);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
}

/** e.g. "+€45.20" / "-€12.00". Assumes `amount` is already in `currency`'s units (not cents). */
export function formatCurrencyResult(amount: number, currency: string): string {
  const sign = amount > 0 ? "+" : amount < 0 ? "-" : "";
  const formatted = Math.abs(amount).toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
  const symbol = currencySymbol(currency);
  return `${sign}${symbol}${formatted}`;
}

function currencySymbol(currency: string): string {
  switch (currency) {
    case "EUR":
      return "€";
    case "USD":
      return "$";
    case "GBP":
      return "£";
    default:
      return `${currency} `;
  }
}

/** Naive local timestamps like "2026-08-25T21:00:00" (no timezone) parse as local time in JS. */
export function parseLocalTimestamp(iso: string): Date {
  return new Date(iso);
}

export function formatSessionDate(iso: string): string {
  return parseLocalTimestamp(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/**
 * A session's clock time, e.g. `10:42 PM`.
 *
 * The space before the meridiem is a non-breaking one. In the Sessions
 * table at the 800px window floor the Start-End cell is only wide enough for
 * one time, so it wraps — and with an ordinary space it broke *inside* a time,
 * turning one range into three lines of "12:30" / "AM-12:30" / "AM". Keeping
 * each time atomic makes it break between them instead, where a reader expects.
 */
export function formatSessionTime(iso: string): string {
  return parseLocalTimestamp(iso)
    .toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    })
    .replace(/\s/g, "\u00a0");
}
