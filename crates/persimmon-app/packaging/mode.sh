#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Which signing the release workflow can do with the secrets it has (D5):
#   ad-hoc        no signing secrets: an ad-hoc signature, no notarisation
#   developer-id  the three signing secrets: Developer ID, not notarised
#   notarise      all six: Developer ID, then notarised and stapled
# A set that's only partly there fails, naming what's missing, so a
# half-configured repository never ships a signed DMG Gatekeeper still
# blocks. An empty variable counts as missing (GitHub passes an absent
# secret as "").
set -euo pipefail
SIGN="APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY"
NOTARISE="APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID"

missing() { local v out=""; for v in $1; do [ -n "${!v:-}" ] || out="$out $v"; done; echo "${out# }"; }
count() { local v n=0; for v in $1; do [ -n "${!v:-}" ] && n=$((n + 1)); done; echo $n; }

sign=$(count "$SIGN") notarise=$(count "$NOTARISE")
if [ "$sign" = 0 ] && [ "$notarise" = 0 ]; then echo ad-hoc; exit 0; fi
gaps=$(missing "$SIGN")
[ "$notarise" -gt 0 ] && gaps=$(echo "$gaps $(missing "$NOTARISE")" | xargs)
if [ -n "$gaps" ]; then echo "mode: missing $gaps"; exit 1; fi
if [ "$notarise" = 3 ]; then echo notarise; else echo developer-id; fi
