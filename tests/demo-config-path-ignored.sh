#!/usr/bin/env bash
# Demonstrates that App::save_prefs / Prefs::save ignore App.config_path and
# always resolve Config::path() (XDG) instead — even when a caller has set
# config_path to point somewhere else entirely (or never set it at all).
# Safe to run repeatedly: XDG_CONFIG_HOME is redirected to a fresh throwaway
# dir for the whole subprocess, so the developer's real config is never
# touched by this script.
#
# This script is a standalone, reviewable artifact for the commit that
# introduces it — check out that commit alone to see the bug reproduced
# before any fix lands. Safe to drop from history before merging if
# maintainers don't want to keep it around for regression checks.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

EVIDENCE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/tuxedo-config-bug-evidence.XXXXXX")"
EVIDENCE_CFG="$EVIDENCE_DIR/tuxedo/config.toml"
echo "== Evidence XDG_CONFIG_HOME: $EVIDENCE_DIR =="

check() {
  local label="$1"
  if [ -f "$EVIDENCE_CFG" ]; then
    echo "  -> config.toml materialized under XDG_CONFIG_HOME ($label):"
    sed 's/^/     /' "$EVIDENCE_CFG"
    return 0
  else
    echo "  -> no file under XDG_CONFIG_HOME ($label)"
    return 1
  fi
}

echo
echo "--- Demo 1: tests/snapshots.rs, cycle_sort() scenes ---"
echo "make_app() sets app.config_path = /tmp/tuxedo-snapshot.toml. If"
echo "config_path were honored, XDG_CONFIG_HOME should see NOTHING."
rm -rf "$EVIDENCE_DIR/tuxedo"
XDG_CONFIG_HOME="$EVIDENCE_DIR" cargo test --test snapshots \
  > "$EVIDENCE_DIR/snapshots-run.log" 2>&1 || true
demo1_bug=0
check "demo 1" && demo1_bug=1

echo
echo "--- Demo 2: src/main.rs, settings_hinted_keys_apply_and_dialog_stays_open ---"
echo "build_app() leaves config_path = None entirely."
rm -rf "$EVIDENCE_DIR/tuxedo"
XDG_CONFIG_HOME="$EVIDENCE_DIR" cargo test --bin tuxedo \
  settings_hinted_keys_apply_and_dialog_stays_open \
  > "$EVIDENCE_DIR/main-run.log" 2>&1 || true
demo2_bug=0
check "demo 2" && demo2_bug=1

echo
echo "== Summary =="
echo "demo 1 (snapshots cycle_sort ignores config_path): $([ $demo1_bug = 1 ] && echo BUG REPRODUCED || echo not reproduced)"
echo "demo 2 (main.rs settings test, config_path=None):  $([ $demo2_bug = 1 ] && echo BUG REPRODUCED || echo not reproduced)"
echo "Logs + evidence retained at: $EVIDENCE_DIR"
