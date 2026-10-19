/**
 * A channel's mana (the server's `app::mana`): a budget the channel spends as messages are
 * posted and earns back over time. The server tells those viewing a channel its level while it
 * is below full, at most twice a second as it is spent; between frames it recharges at a known
 * rate, in ticks of `MANA_TICK_MS`, so the app works it out itself.
 */

/** How often a pool earns back mana, as the server ticks. */
export const MANA_TICK_MS = 200;

/** Below this share of its maximum the bar shows; it hides again once the pool is full. */
export const MANA_SHOW_BELOW = 0.6;

/** A channel's mana as last told: `level` in thousandths of mana, below zero in debt, at `at`. */
export interface ChannelMana {
  level: number;
  max: number;
  /** Mana earned back a second. */
  recharge: number;
  /** When `level` was so, in milliseconds since 1970. */
  at: number;
}

/** The level of `mana` at `now`, in thousandths of mana, as the server's ticks would make it. */
export function manaAt(mana: ChannelMana, now: number): number {
  const ticks = Math.max(0, Math.floor((now - mana.at) / MANA_TICK_MS));
  const perTick = (mana.recharge * 1000 * MANA_TICK_MS) / 1000;
  return Math.min(mana.max * 1000, mana.level + ticks * perTick);
}

/** When `mana` will be full again, in milliseconds since 1970. */
export function manaFullAt(mana: ChannelMana): number {
  const perTick = (mana.recharge * 1000 * MANA_TICK_MS) / 1000;
  if (perTick <= 0) {
    return Number.POSITIVE_INFINITY;
  }
  return mana.at + Math.ceil((mana.max * 1000 - mana.level) / perTick) * MANA_TICK_MS;
}
