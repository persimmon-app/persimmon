// SPDX-License-Identifier: GPL-3.0-or-later
// The fake app binary for the release check's tests: it links libfake.
int fake(void);
int main(void) { return fake(); }
