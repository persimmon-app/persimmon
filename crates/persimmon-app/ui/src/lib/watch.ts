// SPDX-License-Identifier: GPL-3.0-or-later
// What the Watch folders view says about each rule (spec D2, D9).
import { plural } from "./format";
import type { RuleView } from "./types";

function today(n: number, none = "No files today"): string {
  return n ? `${plural(n, "file")} today` : none;
}

/** The rule's line: what it's doing now, else today's count. */
export function statusLine(v: RuleView): string {
  const s = v.status;
  switch (s.state) {
    case "converting": return `Converting ${Math.min(s.done + 1, s.total)} of ${s.total}…`;
    case "missing": return "Folder not found — waiting for it";
    case "unreadable": return s.error.reason;
    case "paused": return "Paused";
    default: return today(v.today);
  }
}

/** The last file the rule couldn't convert, if any. */
export function failedLine(v: RuleView): string | null {
  const f = v.status.lastFailed;
  return f ? `Last failed: ${f.name} — ${f.reason}` : null;
}

/** "2 active · 24 files today", over every rule. */
export function summary(views: RuleView[]): string {
  if (!views.length) return "No rules yet";
  const active = views.filter((v) => v.enabled).length;
  const n = views.reduce((sum, v) => sum + v.today, 0);
  return `${active} active · ${today(n, "no files today")}`;
}

/** Local midnight of `now`, in Unix seconds: where "today" starts. */
export function midnight(now: Date): number {
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}
