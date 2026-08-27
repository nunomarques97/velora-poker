/**
 * PokerStars hand-history setup steps, kept as plain data so this can be
 * updated independently of the onboarding UI if PokerStars changes its menu
 * wording/layout.
 */
export const POKERSTARS_TUTORIAL_STEPS: string[] = [
  "Open the PokerStars client and log in.",
  "Go to Settings from the top menu (gear icon, or Alt key to reveal the menu bar).",
  "Open the \"Hand History\" tab inside Settings.",
  "Make sure \"Save Hand History\" (or equivalent) is enabled.",
  "Note the folder shown as the hand history save location — Velora looks for this automatically, but you can always point it there manually.",
  "Play a hand (real money or play money) and Velora will detect the new file automatically once the folder is configured.",
];

export const POKERSTARS_TUTORIAL_NOTE =
  "Exact menu wording can vary slightly between PokerStars versions/regions — the steps above describe the general flow.";
