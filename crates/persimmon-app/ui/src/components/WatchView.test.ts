// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import WatchView from "./WatchView.svelte";
import { view } from "../test/fixtures";
import type { NotificationsState } from "../lib/types";
import type { ComponentProps } from "svelte";

HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) { this.open = true; };
HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) { this.open = false; };

function props(over: Partial<ComponentProps<typeof WatchView>> = {}) {
  return {
    views: [view({ today: 20 }), view({ id: "r2", folderLabel: "~/Scans", today: 4 })],
    notifications: "allowed" as NotificationsState, startAtLogin: false,
    onNew: vi.fn(), onEdit: vi.fn(), onToggle: vi.fn(), onCopy: vi.fn(), onScan: vi.fn(), onRemove: vi.fn(),
    onStartAtLogin: vi.fn(), onOpenNotifications: vi.fn(), loginError: null,
    ...over,
  };
}

describe("WatchView", () => {
  it("sums up its rules and says they keep working", () => {
    render(WatchView, props());
    expect(screen.getByText("2 active · 24 files today")).toBeTruthy();
    expect(screen.getByText("Rules keep working when this window is closed")).toBeTruthy();
    expect(screen.getAllByRole("switch")).toHaveLength(2);
  });

  it("starts at login from the footer", async () => {
    const p = props();
    render(WatchView, p);
    await fireEvent.click(screen.getByLabelText("Start at login"));
    expect(p.onStartAtLogin).toHaveBeenCalledWith(true);
  });

  it("says when notifications are off, only then", async () => {
    const { unmount } = render(WatchView, props());
    expect(screen.queryByText(/Notifications are off/)).toBeNull();
    unmount();
    const p = props({ notifications: "off" });
    render(WatchView, p);
    expect(screen.getByText(/Notifications are off/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Open System Settings" }));
    expect(p.onOpenNotifications).toHaveBeenCalled();
  });

  it("makes a new rule", async () => {
    const p = props({ views: [] });
    render(WatchView, p);
    expect(screen.getByText("No rules yet")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "New rule" }));
    expect(p.onNew).toHaveBeenCalled();
  });
});
