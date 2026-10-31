#!/usr/bin/env bash
# POSIX mirror of scripts/serve-acceptance.ps1: build the console bundle,
# serve it from `rove-api --web-dist` on one origin, and run the real-API
# Playwright suite against that process — the same form scripts/serve.ps1
# hands a user. Nothing here touches the operator's data root or trust store;
# both are pinned to a scratch directory for the spawned server.
set -euo pipefail

API_ADDR="${ROVE_SERVE_ACCEPTANCE_ADDR:-127.0.0.1:18787}"
SKIP_WEB_BUILD=0
SKIP_CARGO_BUILD=0

while [ $# -gt 0 ]; do
    case "$1" in
        --skip-web-build) SKIP_WEB_BUILD=1; shift ;;
        --skip-cargo-build) SKIP_CARGO_BUILD=1; shift ;;
        --addr) API_ADDR="$2"; shift 2 ;;
        -h|--help)
            echo "usage: $0 [--addr HOST:PORT] [--skip-web-build] [--skip-cargo-build]"
            exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEB_ROOT="$REPO_ROOT/apps/web"
WEB_DIST="$WEB_ROOT/web-dist"

command -v pnpm >/dev/null 2>&1 || { echo "required command 'pnpm' was not found on PATH" >&2; exit 2; }
command -v cargo >/dev/null 2>&1 || { echo "required command 'cargo' was not found on PATH" >&2; exit 2; }

if [ "$SKIP_WEB_BUILD" -eq 0 ] || [ ! -f "$WEB_DIST/index.html" ]; then
    # The real-API spec asserts English copy; the product shell defaults to
    # zh-CN unless ROVE_DEFAULT_LOCALE is baked in at build time.
    echo "building web bundle -> apps/web/web-dist"
    ( cd "$WEB_ROOT" && ROVE_DEFAULT_LOCALE=en-US pnpm build:web )
fi

if [ "$SKIP_CARGO_BUILD" -eq 0 ]; then
    echo "building rove-api"
    ( cd "$REPO_ROOT" && cargo build -p rove-api --bin rove-api )
fi

# `.cargo/config.toml` pins target-dir relative to the repository root, so
# worktree checkouts share one build cache. Ask Cargo where that is instead
# of assuming `$REPO_ROOT/target`.
TARGET_DIR="$(cargo metadata --no-deps --format-version 1 --manifest-path "$REPO_ROOT/Cargo.toml" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug"
API_BIN="$TARGET_DIR/rove-api"
[ -f "$API_BIN" ] || { echo "rove-api binary not found under $TARGET_DIR" >&2; exit 1; }

SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/rove-serve-acceptance-XXXXXXXX")"
WORKSPACE="$SCRATCH/workspace"
DATA_ROOT="$SCRATCH/data"
mkdir -p "$WORKSPACE" "$DATA_ROOT"
LOG_OUT="$SCRATCH/rove-api.out.log"
LOG_ERR="$SCRATCH/rove-api.err.log"
SERVER_PID=""

cleanup() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    rm -rf "$SCRATCH"
}
trap cleanup EXIT

# Isolate the spawned server from the operator's real state: product data, the
# Project Trust store, and the user config root that hosts the provider catalog
# all stay under the scratch directory.
env \
    ROVE_PROVIDER=fake \
    ROVE_MODEL=fake \
    ROVE_DATA_ROOT="$DATA_ROOT" \
    ROVE_CONFIG_ROOT="$DATA_ROOT/config" \
    ROVE_PROJECT_TRUST_STORE="$DATA_ROOT/project-trust.sqlite" \
    "$API_BIN" --addr "$API_ADDR" -C "$WORKSPACE" --web-dist "$WEB_DIST" \
    >"$LOG_OUT" 2>"$LOG_ERR" &
SERVER_PID=$!

HEALTH_URL="http://$API_ADDR/health"
ready=0
for _ in $(seq 1 120); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "rove-api exited before becoming ready:" >&2
        cat "$LOG_ERR" >&2 || true
        exit 1
    fi
    if curl -fsS --max-time 2 "$HEALTH_URL" 2>/dev/null | grep -q '"status" *: *"ok"'; then
        ready=1
        break
    fi
    sleep 0.5
done
[ "$ready" -eq 1 ] || { echo "rove-api did not answer $HEALTH_URL within 60s" >&2; exit 1; }
echo "rove-api serving $WEB_DIST at http://$API_ADDR/"

# The suite shares one product catalog and trust store; keep the browser
# fan-out serial so assertions observe exactly one client's mutations.
( cd "$WEB_ROOT" && \
    ROVE_REAL_API_E2E=1 \
    PLAYWRIGHT_BASE_URL="http://$API_ADDR" \
    ROVE_E2E_WORKERS=1 \
    pnpm exec playwright test tests/e2e/real-api.spec.ts )

echo "serve-form acceptance passed: the hosted bundle drove the real API end to end"
