#!/usr/bin/env bash
# POSIX acceptance entry. Mirrors scripts/product-acceptance.ps1: same check
# table, same report contract. Every status comes from a real exit code and
# anything skipped is recorded as not_run with a reason.
set -uo pipefail

REPORT_PATH="${ROVE_ACCEPTANCE_REPORT:-PRODUCT_ACCEPTANCE_REPORT.json}"
SKIP_WEB=0
SKIP_BROWSER=0
INCLUDE_GATED=0

while [ $# -gt 0 ]; do
    case "$1" in
        --report-path) REPORT_PATH="$2"; shift 2 ;;
        --skip-web) SKIP_WEB=1; shift ;;
        --skip-browser) SKIP_BROWSER=1; shift ;;
        --include-gated) INCLUDE_GATED=1; shift ;;
        -h|--help)
            echo "usage: $0 [--report-path PATH] [--skip-web] [--skip-browser] [--include-gated]"
            exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEB_ROOT="$REPO_ROOT/apps/web"
LOG_DIR="$REPO_ROOT/.rove/acceptance-logs"
rm -rf "$LOG_DIR"
mkdir -p "$LOG_DIR"

case "$REPORT_PATH" in
    /*) ;;
    *) REPORT_PATH="$REPO_ROOT/$REPORT_PATH" ;;
esac

# id|group|description|cwd|required|kind|gate_env|command...
CHECKS=(
    "fmt|gate|Rust formatting|.|1|plain||cargo fmt --all --check"
    "clippy|gate|Rust lints as errors|.|1|plain||cargo clippy --workspace --all-targets -- -D warnings"
    "test-api|G1-G7|API contract suite|.|1|plain||cargo test -p rove-integration-tests --test api -- --test-threads=1"
    "test-mcp|G7|MCP transport and hardening|.|1|plain||cargo test -p rove-integration-tests --test mcp"
    "test-e2e|G1-G4|Engine and planner loop|.|1|plain||cargo test -p rove-integration-tests --test e2e"
    "test-tool-safety|G5|Tool safety boundaries|.|1|plain||cargo test -p rove-integration-tests --test tool_safety"
    "test-product-store|G1-G6|Product store persistence|.|1|plain||cargo test -p rove-api --lib product:: -- --test-threads=1"
    "web-typecheck|G3-G7|Web TypeScript|apps/web|1|web||pnpm typecheck"
    "web-test|G3-G7|Web unit and component tests|apps/web|1|web||pnpm test"
    "web-build|G3-G7|Web production build|apps/web|1|web||pnpm build"
    "web-e2e|G1-G7|Browser-boundary Playwright suites|apps/web|1|browser||pnpm test:e2e"
    "mcp-filesystem-smoke|G7|Real MCP filesystem server smoke|.|0|gated|ROVE_MCP_FILESYSTEM_SMOKE|cargo test -p rove-integration-tests --test mcp mcp_official_filesystem_server_smoke_when_enabled -- --exact --nocapture"
)

json_escape() {
    local s="$1"
    s="${s//\\/\\\\}"
    s="${s//\"/\\\"}"
    s="${s//$'\r'/}"
    s="${s//$'\t'/\\t}"
    s="${s//$'\n'/\\n}"
    printf '%s' "$s"
}

json_string_or_null() {
    if [ -z "$1" ]; then printf 'null'; else printf '"%s"' "$(json_escape "$1")"; fi
}

tool_version() {
    local command="$1"; shift
    if ! command -v "$command" >/dev/null 2>&1; then
        printf ''
        return
    fi
    "$command" "$@" 2>/dev/null | head -n 1 | tr -d '\r'
}

output_tail() {
    local path="$1" lines="$2"
    [ -f "$path" ] || return 0
    tail -n "$lines" "$path" 2>/dev/null | tr -d '\r'
}

CHECK_JSON=()
PASSED=0
FAILED=0
NOT_RUN=0
REQUIRED_NOT_RUN=0
FAIL_SUMMARY=()
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
STARTED_EPOCH="$(date +%s)"

for entry in "${CHECKS[@]}"; do
    IFS='|' read -r id group description rel_cwd required kind gate_env command_line <<< "$entry"
    cwd="$REPO_ROOT"
    [ "$rel_cwd" != "." ] && cwd="$REPO_ROOT/$rel_cwd"
    binary="${command_line%% *}"

    skip_reason=""
    if [ "$kind" = "web" ] && [ "$SKIP_WEB" -eq 1 ]; then
        skip_reason="--skip-web was requested"
    elif [ "$kind" = "browser" ] && { [ "$SKIP_BROWSER" -eq 1 ] || [ "$SKIP_WEB" -eq 1 ]; }; then
        skip_reason="--skip-browser or --skip-web was requested"
    elif [ "$kind" = "gated" ] && [ "$INCLUDE_GATED" -eq 0 ]; then
        skip_reason="gated check; pass --include-gated to run it"
    elif [ "$kind" = "gated" ] && [ -n "$gate_env" ] && [ -z "${!gate_env:-}" ]; then
        skip_reason="gate variable $gate_env is not set"
    elif ! command -v "$binary" >/dev/null 2>&1; then
        skip_reason="required command '$binary' was not found on PATH"
    fi

    required_bool=false
    [ "$required" = "1" ] && required_bool=true

    if [ -n "$skip_reason" ]; then
        echo "not_run  $id : $skip_reason"
        NOT_RUN=$((NOT_RUN + 1))
        if [ "$required" = "1" ]; then
            REQUIRED_NOT_RUN=$((REQUIRED_NOT_RUN + 1))
            FAIL_SUMMARY+=("  NOT RUN $id: $skip_reason")
        fi
        CHECK_JSON+=("$(printf '{"id":"%s","group":"%s","description":"%s","command":"%s","working_directory":"%s","required":%s,"status":"not_run","reason":"%s","exit_code":null,"duration_seconds":null,"output_tail":[]}' \
            "$(json_escape "$id")" "$(json_escape "$group")" "$(json_escape "$description")" \
            "$(json_escape "$command_line")" "$(json_escape "$rel_cwd")" "$required_bool" "$(json_escape "$skip_reason")")")
        continue
    fi

    stdout_log="$LOG_DIR/$id.out.log"
    stderr_log="$LOG_DIR/$id.err.log"
    echo "running  $id : $command_line"

    check_start="$(date +%s)"
    ( cd "$cwd" && eval "$command_line" ) >"$stdout_log" 2>"$stderr_log"
    exit_code=$?
    duration=$(( $(date +%s) - check_start ))

    # Status is derived only from a real exit code. There is no default pass.
    if [ "$exit_code" -eq 0 ]; then
        status="pass"
        PASSED=$((PASSED + 1))
        tail_text="$(output_tail "$stdout_log" 15)"
    else
        status="fail"
        FAILED=$((FAILED + 1))
        FAIL_SUMMARY+=("  FAILED  $id (exit $exit_code)")
        tail_text="$(output_tail "$stdout_log" 40)"$'\n'"$(output_tail "$stderr_log" 40)"
    fi

    echo "$status  $id (exit $exit_code, ${duration}s)"

    tail_json=""
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        [ -n "$tail_json" ] && tail_json="$tail_json,"
        tail_json="$tail_json\"$(json_escape "$line")\""
    done <<< "$tail_text"

    CHECK_JSON+=("$(printf '{"id":"%s","group":"%s","description":"%s","command":"%s","working_directory":"%s","required":%s,"status":"%s","reason":null,"exit_code":%s,"duration_seconds":%s,"output_tail":[%s]}' \
        "$(json_escape "$id")" "$(json_escape "$group")" "$(json_escape "$description")" \
        "$(json_escape "$command_line")" "$(json_escape "$rel_cwd")" "$required_bool" \
        "$status" "$exit_code" "$duration" "$tail_json")")
done

# PASS requires zero failures and zero unrun required checks.
VERDICT="FAIL"
if [ "$FAILED" -eq 0 ] && [ "$REQUIRED_NOT_RUN" -eq 0 ]; then
    VERDICT="PASS"
fi

BRANCH="$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
DIRTY_OUTPUT="$(git -C "$REPO_ROOT" status --porcelain 2>/dev/null || printf '')"
DIRTY_COUNT="$(printf '%s' "$DIRTY_OUTPUT" | grep -c . || true)"
DIRTY=false
[ "$DIRTY_COUNT" -gt 0 ] && DIRTY=true
TOTAL=${#CHECKS[@]}
TOTAL_DURATION=$(( $(date +%s) - STARTED_EPOCH ))

{
    printf '{\n'
    printf '  "schema_version": 1,\n'
    printf '  "report": "product-acceptance",\n'
    printf '  "generated_at": "%s",\n' "$STARTED_AT"
    printf '  "duration_seconds": %s,\n' "$TOTAL_DURATION"
    printf '  "verdict": "%s",\n' "$VERDICT"
    printf '  "source": {\n'
    printf '    "branch": "%s",\n' "$(json_escape "$BRANCH")"
    printf '    "commit": "%s",\n' "$(json_escape "$COMMIT")"
    printf '    "dirty": %s,\n' "$DIRTY"
    printf '    "dirty_file_count": %s\n' "$DIRTY_COUNT"
    printf '  },\n'
    printf '  "environment": {\n'
    printf '    "os": "%s",\n' "$(json_escape "$(uname -sr 2>/dev/null || echo unknown)")"
    printf '    "cargo": %s,\n' "$(json_string_or_null "$(tool_version cargo --version)")"
    printf '    "rustc": %s,\n' "$(json_string_or_null "$(tool_version rustc --version)")"
    printf '    "node": %s,\n' "$(json_string_or_null "$(tool_version node --version)")"
    printf '    "pnpm": %s\n' "$(json_string_or_null "$(tool_version pnpm --version)")"
    printf '  },\n'
    printf '  "totals": {\n'
    printf '    "total": %s,\n' "$TOTAL"
    printf '    "passed": %s,\n' "$PASSED"
    printf '    "failed": %s,\n' "$FAILED"
    printf '    "not_run": %s,\n' "$NOT_RUN"
    printf '    "required_not_run": %s\n' "$REQUIRED_NOT_RUN"
    printf '  },\n'
    printf '  "checks": [\n'
    for i in "${!CHECK_JSON[@]}"; do
        printf '    %s' "${CHECK_JSON[$i]}"
        if [ "$i" -lt $((${#CHECK_JSON[@]} - 1)) ]; then printf ','; fi
        printf '\n'
    done
    printf '  ]\n'
    printf '}\n'
} > "$REPORT_PATH"

echo ""
echo "verdict: $VERDICT ($PASSED passed, $FAILED failed, $NOT_RUN not run)"
echo "report:  $REPORT_PATH"
echo "logs:    $LOG_DIR"

if [ "$VERDICT" != "PASS" ]; then
    for line in "${FAIL_SUMMARY[@]}"; do echo "$line"; done
    exit 1
fi
exit 0
