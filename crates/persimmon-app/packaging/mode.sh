#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Which signing the release workflow can do with the secrets it has (D5):
#   ad-hoc    no Apple secrets: an ad-hoc signature, no notarisation
#   notarise  all six: Developer ID, then notarised and stapled
# Anything in between fails, naming what's missing on stderr, so a
# half-configured repository never ships a signed DMG that Gatekeeper still
# blocks. An empty variable counts as missing (GitHub passes an absent
# secret as "").
set -euo pipefail
ALL="APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID"
missing="" present=0
for v in $ALL; do
  if [ -n "${!v:-}" ]; then present=$((present + 1)); else missing="$missing $v"; fi
done
if [ "$present" = 0 ]; then echo ad-hoc; exit 0; fi
if [ -n "$missing" ]; then echo "mode: missing${missing}" >&2; exit 1; fi
echo notarise
