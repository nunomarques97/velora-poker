// Tilt episodes of the profiles that have a `tilt` block, shared by every
// format of the simulator.

/**
 * Tilt (profiles' `tilt` block): after a loss of at least `triggerLossBb`,
 * the player plays his tilt behaviour for his next `hands` hands; a new
 * episode can start only after `cooldownHands` more hands.
 */
export function trackTilt(who, net, handId, state, tiltedThisHand, episodes, bb, played) {
  const config = who.profile.tilt;
  if (!config || !state) return;
  if (tiltedThisHand.includes(who.name)) {
    state.left -= 1;
    const episode = episodes.findLast((e) => e.player === who.name);
    episode.handIds.push(String(handId));
  }
  if (state.cooldown > 0) state.cooldown -= 1;
  if (state.left === 0 && state.cooldown === 0 && net <= -config.triggerLossBb * bb) {
    state.left = config.hands;
    state.cooldown = config.hands + config.cooldownHands;
    // `handsBefore`: his hands up to and including the big loss.
    episodes.push({ player: who.name, profile: who.profile.id, bigLossHandId: String(handId), handsBefore: played, lossBb: -net / bb, handIds: [] });
  }
}
