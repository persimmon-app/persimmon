// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import EnginesPane from "./EnginesPane.svelte";
import PrivacyPane from "./PrivacyPane.svelte";
import AboutPane from "./AboutPane.svelte";
import StatusBar from "./StatusBar.svelte";

describe("the Settings panes", () => {
  it("Engines names the FFmpeg in use", () => {
    render(EnginesPane, { engines: { ffmpeg: { path: "/opt/homebrew/bin/ffmpeg", version: "8.1" }, ffmpegError: null, images: true, rar: true, version: "0.1.0" } });
    expect(screen.getByText("8.1")).toBeTruthy();
    expect(screen.getByText("/opt/homebrew/bin/ffmpeg")).toBeTruthy();
  });

  it("Engines says, in the app's words, when FFmpeg is missing", () => {
    render(EnginesPane, { engines: { ffmpeg: null, images: true, rar: false, version: "0.1.0",
      ffmpegError: { code: "BACKEND_MISSING", file: null, reason: "ffmpeg isn't installed", fix: "Persimmon's FFmpeg is missing or damaged. Reinstall Persimmon." } } });
    expect(screen.getByText(/Reinstall Persimmon/)).toBeTruthy();
    expect(screen.getByText("Not in this build")).toBeTruthy();
  });

  it("Privacy says there is no network code", () => {
    render(PrivacyPane);
    expect(screen.getByText(/Persimmon has no network code/)).toBeTruthy();
    expect(screen.getByText("Updates come from the Releases page.")).toBeTruthy();
  });

  it("About explains when the licence list isn't here yet", async () => {
    const onLicences = vi.fn().mockResolvedValue(false);
    render(AboutPane, { version: "0.1.0", onLicences, onSources: () => {} });
    expect(screen.getByText("GPL-3.0-or-later")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Licences…" }));
    expect(await screen.findByText("The full licence list comes with the released app.")).toBeTruthy();
  });

  it("About links to the sources", async () => {
    const onSources = vi.fn();
    render(AboutPane, { version: "0.2.0", onLicences: vi.fn().mockResolvedValue(true), onSources });
    expect(screen.getByText("The sources of FFmpeg, libvips and the libraries they use, with FFmpeg's configure line, are attached to this version's release.")).toBeTruthy();
    expect(screen.queryByText("FFmpeg's source arrives with the packaged app.")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Source code…" }));
    expect(onSources).toHaveBeenCalledOnce();
  });

  it("the status bar offers to undo the last batch only while it can", async () => {
    const onUndoLast = vi.fn();
    const { rerender } = render(StatusBar, { rows: [], message: null, output: "beside original", onOutput: () => {}, undoable: false, onUndoLast });
    expect(screen.queryByRole("button", { name: /Undo last batch/ })).toBeNull();
    await rerender({ rows: [], message: null, output: "beside original", onOutput: () => {}, undoable: true, onUndoLast });
    await fireEvent.click(screen.getByRole("button", { name: /Undo last batch/ }));
    expect(onUndoLast).toHaveBeenCalled();
  });
});
