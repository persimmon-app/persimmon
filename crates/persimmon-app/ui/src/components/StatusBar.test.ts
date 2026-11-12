// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/svelte";
import StatusBar from "./StatusBar.svelte";
import type { RowView } from "../lib/types";

const r = (id: number, state: object) =>
  ({ id, path: "", name: "", from: "png", to: "jpeg", op: "convert", size: 1, output: null, ...state }) as RowView;

describe("StatusBar", () => {
  it("counts rows by state", () => {
    render(StatusBar, {
      rows: [
        r(1, { state: "done", outputSize: 1 }), r(2, { state: "done", outputSize: 1 }),
        r(3, { state: "running", fraction: 0.1 }), r(4, { state: "queued" }), r(5, { state: "queued" }),
        r(6, { state: "failed", error: { code: "X", file: null, reason: "r", fix: null } }),
      ],
      message: null,
    });
    expect(screen.getByText("2 done")).toBeTruthy();
    expect(screen.getByText("1 running")).toBeTruthy();
    expect(screen.getByText("2 queued")).toBeTruthy();
    expect(screen.getByText("1 failed")).toBeTruthy();
    expect(screen.getByText("Output → beside original")).toBeTruthy();
  });

  it("shows a message, such as rows a format skipped", () => {
    render(StatusBar, { rows: [], message: "2 files can't become MP4" });
    expect(screen.getByText("2 files can't become MP4")).toBeTruthy();
    expect(screen.getByText("Queue empty")).toBeTruthy();
  });
});
