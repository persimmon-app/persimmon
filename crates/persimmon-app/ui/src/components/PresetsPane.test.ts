// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import PresetsPane from "./PresetsPane.svelte";
import { presets } from "../test/fixtures";

describe("PresetsPane", () => {
  it("previews the command as fields change, and saves a built-in as a copy", async () => {
    const onSave = vi.fn().mockResolvedValue(null);
    render(PresetsPane, { presets, onSave, onDelete: vi.fn() });
    await fireEvent.click(screen.getByRole("button", { name: "Web image" }));
    expect(screen.getByText("prsm convert <files> --to webp --quality medium --max-size 2048 --strip-metadata")).toBeTruthy();
    await fireEvent.input(screen.getByLabelText("Max size (px)"), { target: { value: "1024" } });
    await tick();
    expect(screen.getByText("prsm convert <files> --to webp --quality medium --max-size 1024 --strip-metadata")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Save as copy" }));
    expect(onSave.mock.calls[0][0].options.maxSize).toBe(1024);
  });

  it("shows why a preset can't be saved", async () => {
    const onSave = vi.fn().mockResolvedValue("A size limit is for images and video");
    render(PresetsPane, { presets, onSave, onDelete: vi.fn() });
    await fireEvent.click(screen.getByRole("button", { name: "Thumbs" }));
    await fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await tick();
    expect(await screen.findByText("A size limit is for images and video")).toBeTruthy();
  });

  it("deletes only your own presets", async () => {
    const onDelete = vi.fn().mockResolvedValue(null);
    render(PresetsPane, { presets, onSave: vi.fn(), onDelete });
    await fireEvent.click(screen.getByRole("button", { name: "Web image" }));
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Thumbs" }));
    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(onDelete).toHaveBeenCalledWith("thumbs");
  });
});
