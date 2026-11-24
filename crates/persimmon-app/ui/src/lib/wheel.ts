// SPDX-License-Identifier: GPL-3.0-or-later
// The wheel's geometry, mirroring wheel.rs for hover and drawing only: the
// app decides what a drop or a click means (wheel_drop, wheel_pick).

/** The wheel's diameter, the hub's radius, and how many segments. */
export const SIZE = 320;
export const HUB = 64;
export const SEGMENTS = 6;
const C = SIZE / 2;
const STEP = 360 / SEGMENTS;

export type Hit = "hub" | "outside" | number;

/** Where (x, y), in points from the wheel's top-left, lands: segment 0 is
 * centred at 12 o'clock, then clockwise. */
export function hit(x: number, y: number): Hit {
  const dx = x - C, dy = y - C;
  const r = Math.hypot(dx, dy);
  if (r <= HUB) return "hub";
  if (r > C) return "outside";
  const deg = (((Math.atan2(dx, -dy) * 180) / Math.PI) % 360 + 360) % 360;
  return Math.floor((deg + STEP / 2) / STEP) % SEGMENTS;
}

/** A point at radius `r` and `deg` clockwise from 12 o'clock. */
const at = (r: number, deg: number) => {
  const a = (deg * Math.PI) / 180;
  return { x: C + r * Math.sin(a), y: C - r * Math.cos(a) };
};

/** Segment `i` as an SVG path: the ring between the hub and the rim. */
export function sector(i: number): string {
  const from = i * STEP - STEP / 2, to = from + STEP, outer = C - 1, inner = HUB;
  const [a, b, c, d] = [at(outer, from), at(outer, to), at(inner, to), at(inner, from)];
  const n = (v: number) => v.toFixed(2);
  return `M ${n(a.x)} ${n(a.y)} A ${outer} ${outer} 0 0 1 ${n(b.x)} ${n(b.y)} L ${n(c.x)} ${n(c.y)} A ${inner} ${inner} 0 0 0 ${n(d.x)} ${n(d.y)} Z`;
}

/** Where segment `i`'s badge sits: midway across the ring. */
export const badgeAt = (i: number) => at((HUB + C) / 2, i * STEP);
