#!/usr/bin/env bash
# Runs all quality gates for the workspace. All gates must pass before a commit.
#
# Usage: bash scripts/check.sh [--rust-only | --ui-only]
#
# Note: the Tauri crate does not compile when apps/desktop/ui/dist is missing.
# "--rust-only" does not build the UI. Use the default mode in a new checkout.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UI_DIR="$ROOT/apps/desktop/ui"
MODE="${1:-all}"

run_rust() {
  # Use the rustfmt of the pinned toolchain. Another rustfmt earlier in PATH
  # (for example one installed by an editor) can be a different version or
  # a binary for a different CPU.
  if command -v rustup >/dev/null 2>&1; then
    RUSTFMT="$(rustup which rustfmt)"
    export RUSTFMT
  fi
  echo "==> cargo fmt"
  cargo fmt --all --check
  echo "==> cargo clippy"
  cargo clippy --workspace --all-targets -- -D warnings
  echo "==> cargo test"
  cargo test --workspace
}

ensure_node_modules() {
  if [[ ! -d "$UI_DIR/node_modules" ]]; then
    echo "==> npm ci (node_modules is missing)"
    npm --prefix "$UI_DIR" ci
  fi
}

run_ui() {
  ensure_node_modules
  echo "==> npm run check"
  npm --prefix "$UI_DIR" run check
  echo "==> npm test"
  npm --prefix "$UI_DIR" test
}

cd "$ROOT"
case "$MODE" in
  --rust-only) run_rust ;;
  --ui-only) run_ui ;;
  all)
    # The Tauri build checks that the UI output folder exists, so build the UI first.
    if [[ ! -d "$UI_DIR/dist" ]]; then
      ensure_node_modules
      echo "==> npm run build (dist is missing)"
      npm --prefix "$UI_DIR" run build
    fi
    run_ui
    run_rust
    ;;
  *)
    echo "Unknown option: $MODE" >&2
    exit 2
    ;;
esac
echo "All checks passed."
