# Research: OpenCode Go Contracts

Date: 2026-06-02
Status: blocked

## Summary

OpenCode Go provider/model identity is verified, and the official console computes the exact quota fields we want. Implementation should not proceed yet because the verified quota source is an authenticated console server query, not a documented CLI command or public endpoint this desktop app can call safely.

## Verified Facts

- Provider/model ID format: OpenCode Go models use `opencode-go/<model-id>`, for example `opencode-go/kimi-k2.6`.
- Official limits: Go has a rolling 5-hour window, a weekly window, and a monthly window. The requested UI only needs weekly and monthly sections.
- Limits are cost based, not request-count based: weekly `$30`, monthly `$60`, and rolling 5-hour `$12`.
- CLI auth docs say credentials are stored at `~/.local/share/opencode/auth.json`; locally, `opencode providers` is an alias for auth management.
- Local `opencode stats` only aggregates local sessions from the OpenCode database and reports cost/token totals. It does not report Go quota percentages or reset times.
- Console source has the desired shape:
  - `weeklyUsage: { status, resetInSec, usagePercent }`
  - `monthlyUsage: { status, resetInSec, usagePercent }`
- Console source derives those from `LiteTable.weeklyUsage`, `LiteTable.monthlyUsage`, `LiteTable.timeWeeklyUpdated`, `LiteTable.timeMonthlyUpdated`, `LiteTable.timeCreated`, and `LiteData.getLimits()`.

## Sources

- OpenCode Go docs: https://dev.opencode.ai/docs/go/
- OpenCode CLI docs: https://opencode.ai/docs/cli/
- OpenCode source: `packages/opencode/src/cli/cmd/stats.ts`
- OpenCode source: `packages/console/app/src/routes/workspace/[id]/go/lite-section.tsx`
- OpenCode source: `packages/console/core/src/subscription.ts`
- Local command: `opencode stats --help`

## Decision

Do not implement OpenCode Go four-bar usage yet. The app cannot honestly render weekly/monthly usage percent and remaining time until one of these is chosen:

1. Use an official/detected console endpoint with user-approved authentication.
2. Ship only provider/auth detection now and hide/disable Go usage bars until quota data is available.
3. Ask OpenCode upstream for a documented quota API or CLI output.

## Unresolved Questions

- Should this app integrate with the authenticated OpenCode console, or should Go support be limited to detection until OpenCode exposes a stable quota API?
- Do we still want to continue Phase 2/4 now for the provider terminology rename, while deferring Phase 3?
