// SPDX-License-Identifier: GPL-3.0-or-later
// The wheel's geometry, mirroring wheel.rs for hover and drawing only: the
// app decides what a drop or a click means (wheel_drop, wheel_pick).

/** The wheel's diameter, the hub's radius, and the most segments. */
export const SIZE = 320;
export const HUB = 64;
export const SEGMENTS = 6;
const C = SIZE / 2;

export type Hit = "hub" | "outside" | number;

/** Where (x, y), in points from the wheel's top-left, lands on a ring
 * shared by `n` formats (at most six): segment 0 is centred at 12
 * o'clock, then clockwise. With no formats the ring means nothing. */
export function hit(x: number, y: number, n: number): Hit {
  const count = Math.min(n, SEGMENTS);
  const dx = x - C, dy = y - C;
  const r = Math.hypot(dx, dy);
  if (r <= HUB) return "hub";
  if (r > C || count === 0) return "outside";
  const step = 360 / count;
  const deg = (((Math.atan2(dx, -dy) * 180) / Math.PI) % 360 + 360) % 360;
  return Math.floor((deg + step / 2) / step) % count;
}

/** A point at radius `r` and `deg` clockwise from 12 o'clock. */
const at = (r: number, deg: number) => {
  const a = (deg * Math.PI) / 180;
  return { x: C + r * Math.sin(a), y: C - r * Math.cos(a) };
};
const n2 = (v: number) => v.toFixed(2);

/** Segment `i` of `n` as an SVG path: the ring between the hub and the
 * rim. One format gets the whole ring, drawn as two half arcs each way
 * (an arc can't end where it starts). */
export function sector(i: number, n: number): string {
  const outer = C - 1, inner = HUB;
  if (n <= 1) {
    const [top, bottom, itop, ibottom] = [at(outer, 0), at(outer, 180), at(inner, 0), at(inner, 180)];
    return `M ${n2(top.x)} ${n2(top.y)} A ${outer} ${outer} 0 0 1 ${n2(bottom.x)} ${n2(bottom.y)} A ${outer} ${outer} 0 0 1 ${n2(top.x)} ${n2(top.y)} Z `
      + `M ${n2(itop.x)} ${n2(itop.y)} A ${inner} ${inner} 0 0 0 ${n2(ibottom.x)} ${n2(ibottom.y)} A ${inner} ${inner} 0 0 0 ${n2(itop.x)} ${n2(itop.y)} Z`;
  }
  const step = 360 / Math.min(n, SEGMENTS);
  const from = i * step - step / 2, to = from + step;
  const large = step > 180 ? 1 : 0;
  const [a, b, c, d] = [at(outer, from), at(outer, to), at(inner, to), at(inner, from)];
  return `M ${n2(a.x)} ${n2(a.y)} A ${outer} ${outer} 0 ${large} 1 ${n2(b.x)} ${n2(b.y)} L ${n2(c.x)} ${n2(c.y)} A ${inner} ${inner} 0 ${large} 0 ${n2(d.x)} ${n2(d.y)} Z`;
}

/** Where segment `i` of `n`'s badge sits: midway across the ring. */
export const badgeAt = (i: number, n: number) => at((HUB + C) / 2, (i * 360) / Math.min(Math.max(n, 1), SEGMENTS));
