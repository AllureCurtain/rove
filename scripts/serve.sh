#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
API_ADDR="${ROVE_API_BIND_ADDR:-127.0.0.1:8787}"
WORKSPACE="${ROVE_DEV_WORKSPACE:-$REPO_ROOT}"
SKIP_WEB_BUILD=0
USE_PROVIDER=0

usage() {
    echo "usage: $0 [--addr HOST:PORT] [--workspace DIR] [--provider] [--skip-web-build]"
    echo "Starts the browser console and API on one origin. Defaults to the offline demo."
    echo "--provider keeps your configured provider environment; credentials are never printed."
}

while [ $# -gt 0 ]; do
    case "$1" in
        --addr|--workspace)
            [ $# -ge 2 ] && [ -n "$2" ] || { echo "missing value for $1" >&2; exit 2; }
            case "$1" in
                --addr) API_ADDR="$2" ;;
                --workspace) WORKSPACE="$2" ;;
            esac
            shift 2 ;;
        --provider) USE_PROVIDER=1; shift ;;
        --skip-web-build) SKIP_WEB_BUILD=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

command -v cargo >/dev/null 2>&1 || { echo "required command 'cargo' was not found on PATH" >&2; exit 2; }
[ -d "$WORKSPACE" ] || { echo "workspace must be an existing directory" >&2; exit 2; }
WORKSPACE="$(cd "$WORKSPACE" && pwd -P)"
WEB_DIST="$REPO_ROOT/apps/web/web-dist"

if [ "$SKIP_WEB_BUILD" -eq 0 ] || [ ! -f "$WEB_DIST/index.html" ]; then
    command -v pnpm >/dev/null 2>&1 || { echo "required command 'pnpm' was not found on PATH" >&2; exit 2; }
    echo "building web bundle -> apps/web/web-dist"
    (cd "$REPO_ROOT/apps/web" && pnpm build:web)
fi

if [ "$USE_PROVIDER" -eq 0 ]; then
    export ROVE_PROVIDER=fake
    export ROVE_MODEL=fake
fi

echo "rove console: http://$API_ADDR/"
echo "workspace:    $WORKSPACE"
cd "$REPO_ROOT"
exec cargo run -p rove-api --bin rove-api -- --addr "$API_ADDR" -C "$WORKSPACE" --web-dist "$WEB_DIST"
