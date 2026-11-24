// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import EmptyState from "./EmptyState.svelte";

describe("EmptyState", () => {
  it("invites drops and offers Choose files", async () => {
    const onAdd = vi.fn();
    render(EmptyState, { onAdd });
    expect(screen.getByText("Drop files here, or folders to combine")).toBeTruthy();
    expect(screen.getByText("Images, video, audio, PDF and archives")).toBeTruthy();
    expect(screen.getByText("prsm convert a.mov --to mp4")).toBeTruthy();
    expect(screen.getByText("Drop window")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: /Choose files/ }));
    expect(onAdd).toHaveBeenCalled();
  });

  it("has a card for Finder's Services", () => {
    render(EmptyState, { onAdd: vi.fn() });
    expect(screen.getByText("Right-click → Services → Convert with Persimmon")).toBeTruthy();
  });

  it("shows the wheel tip only when asked, with Turn on and Dismiss", async () => {
    const tip = "New: hold ⇧ while dragging files to pick a format at the pointer";
    render(EmptyState, { onAdd: vi.fn() });
    expect(screen.queryByText(tip)).toBeNull();
    const onTurnOn = vi.fn();
    const onDismissTip = vi.fn();
    render(EmptyState, { onAdd: vi.fn(), showTip: true, onTurnOn, onDismissTip });
    expect(screen.getByText(tip)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Turn on" }));
    expect(onTurnOn).toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onDismissTip).toHaveBeenCalled();
  });
});
