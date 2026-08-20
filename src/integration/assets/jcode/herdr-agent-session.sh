#!/bin/sh
# managed by herdr; reinstalling the integration replaces this file.
# HERDR_INTEGRATION_ID=jcode
# HERDR_INTEGRATION_VERSION=1

[ "${JCODE_HOOK_EVENT:-}" = "session_start" ] || exit 0
[ "${HERDR_ENV:-}" = "1" ] || exit 0
[ -n "${HERDR_PANE_ID:-}" ] || exit 0
[ -n "${HERDR_SOCKET_PATH:-}" ] || exit 0
[ -n "${JCODE_HOOK_SESSION_ID:-}" ] || exit 0
if [ -n "${HERDR_BIN_PATH:-}" ]; then
    [ -x "$HERDR_BIN_PATH" ] || exit 0
else
    command -v herdr >/dev/null 2>&1 || exit 0
fi

herdr_source="startup"
case "${JCODE_HOOK_SOURCE:-}" in
    resume) herdr_source="resume" ;;
    create|attach|"") herdr_source="startup" ;;
esac
seq="$(date +%s%N 2>/dev/null || date +%s)"
herdr_bin="${HERDR_BIN_PATH:-herdr}"

set -- pane report-agent-session "$HERDR_PANE_ID" \
    --source herdr:jcode --agent jcode \
    --agent-session-id "$JCODE_HOOK_SESSION_ID" \
    --seq "$seq" \
    --session-start-source "$herdr_source"
[ -z "${JCODE_HOOK_SESSION_NAME:-}" ] || set -- "$@" --agent-session-name "$JCODE_HOOK_SESSION_NAME"
[ -z "${JCODE_HOOK_SESSION_ICON:-}" ] || set -- "$@" --agent-session-icon "$JCODE_HOOK_SESSION_ICON"
"$herdr_bin" "$@" >/dev/null 2>&1 || true
