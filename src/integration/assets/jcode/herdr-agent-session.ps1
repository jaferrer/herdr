# installed by herdr
# managed by herdr; reinstalling or updating the integration overwrites this file.
# HERDR_INTEGRATION_ID=jcode
# HERDR_INTEGRATION_VERSION=1

if ($env:JCODE_HOOK_EVENT -ne "session_start") { exit 0 }
if ($env:HERDR_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:HERDR_PANE_ID)) { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:HERDR_SOCKET_PATH)) { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:JCODE_HOOK_SESSION_ID)) { exit 0 }

$source = if ($env:JCODE_HOOK_SOURCE -eq "resume") { "resume" } else { "startup" }
$seq = [DateTime]::UtcNow.Ticks
$herdr = if ([string]::IsNullOrWhiteSpace($env:HERDR_BIN_PATH)) { "herdr" } else { $env:HERDR_BIN_PATH }
$commandArgs = @(
    "pane", "report-agent-session", $env:HERDR_PANE_ID,
    "--source", "herdr:jcode", "--agent", "jcode",
    "--agent-session-id", $env:JCODE_HOOK_SESSION_ID,
    "--seq", [string]$seq,
    "--session-start-source", $source
)
if (-not [string]::IsNullOrWhiteSpace($env:JCODE_HOOK_SESSION_NAME)) {
    $commandArgs += @("--agent-session-name", $env:JCODE_HOOK_SESSION_NAME)
}
if (-not [string]::IsNullOrWhiteSpace($env:JCODE_HOOK_SESSION_ICON)) {
    $commandArgs += @("--agent-session-icon", $env:JCODE_HOOK_SESSION_ICON)
}
try {
    & $herdr @commandArgs 2>$null | Out-Null
} catch {
}
