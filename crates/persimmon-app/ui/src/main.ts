// SPDX-License-Identifier: GPL-3.0-or-later
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "./tokens.css";
import { mount } from "svelte";
import App from "./App.svelte";

if (import.meta.env.DEV) {
  const { installDevMock } = await import("./lib/devmock");
  installDevMock();
}

const target = document.getElementById("app");
if (!target) throw new Error("no #app element");
export default mount(App, { target });
