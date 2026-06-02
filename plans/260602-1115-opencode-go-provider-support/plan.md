---
title: OpenCode Go provider support and providers terminology
description: >-
  Add OpenCode Go as a usage provider and rename user-facing Models wording to
  Providers.
status: in-progress
priority: P2
branch: main
tags:
  - providers
  - opencode-go
  - usage
  - ui-copy
blockedBy: []
blocks: []
created: '2026-06-02T04:20:10.983Z'
createdBy: 'ck:plan'
source: skill
---

# OpenCode Go Provider Support And Providers Terminology

## Overview

Add OpenCode Go as a third selectable usage provider beside Claude Code and Codex. Rename the app's user-facing "Models" menu/copy to "Providers" because the app chooses quota sources/services, not individual model IDs.

I agree with the rename. OpenCode itself still has a `/models` command, but this app's menu toggles Claude Code, Codex, and OpenCode Go providers. Internals already use `ProviderId`, so the naming is conceptually aligned.

## Codebase Findings

- Rust Win32 app; usage abstraction lives under `src/usage/*`.
- Current extension points: `UsageProvider`, `ProviderId`, `Registry`, `CredentialSource`, `RefreshHint`.
- Two-provider assumptions remain in `src/settings.rs`, `src/app.rs`, `src/tray/mod.rs`, `src/tray/badge.rs`, `src/panel.rs`, `src/usage_color.rs`, i18n TOMLs, and README.
- OpenCode official docs say Go is configured as an OpenCode provider, uses `/connect`, stores credentials in `~/.local/share/opencode/auth.json`, has 5h/weekly/monthly dollar-value limits, and current usage is visible in the console.
- User decision: OpenCode Go should display four bars in two parts: upper part is weekly usage, lower part is monthly usage. Each part has the same two concepts as current Claude/Codex: usage percent and remaining time. Keep Claude/Codex on their existing two-bar presentation unless a separate UI redesign decides otherwise.
- OpenCode docs expose model endpoints and `https://opencode.ai/zen/go/v1/models`, but do not document a stable usage endpoint. Do not fake usage percentages.
- Phase 1 found the OpenCode console computes `weeklyUsage` and `monthlyUsage` with `usagePercent` and `resetInSec`, but only through an authenticated console server query. `opencode stats` is local cost/token history and cannot drive the requested Go bars.

## Related Plans

- `plans/260516-0707-cleanroom-rewrite/plan.md`: introduced current provider abstraction; related, no blocking dependency.
- `plans/260523-ui-ux-improvement-plan/plan.md`: overlaps labels/tooltips; related, no blocking dependency.

## Phases

| Phase | Name | Status |
|-------|------|--------|
| 1 | [Research OpenCode Go contracts](./phase-01-research-opencode-go-contracts.md) | Blocked |
| 2 | [Refactor provider metadata and settings](./phase-02-refactor-provider-metadata-and-settings.md) | Completed |
| 3 | [Implement OpenCode Go provider](./phase-03-implement-opencode-go-provider.md) | Pending |
| 4 | [Rename Models UI to Providers](./phase-04-rename-models-ui-to-providers.md) | In Progress |
| 5 | [Verify and document](./phase-05-verify-and-document.md) | Pending |

## Dependencies

Phase 1 gates Phase 3. If no stable OpenCode Go usage source is found, pause before Phase 3 and ask user whether to defer usage support or ship a limited connectivity/auth detector.

Phase 2 is implemented. Phase 4 provider wording is implemented; OpenCode Go remains listed but disabled until Phase 1's quota-source decision is resolved.

## Success Criteria

- OpenCode Go can be enabled/disabled independently without breaking Claude Code or Codex.
- Existing settings migrate safely; at least one provider remains enabled.
- UI and README say "Providers" where the app means selectable services.
- No guessed OpenCode Go usage. Weekly/monthly percent and reset-time data comes from verified source, or feature pauses.
- `cargo test` and `cargo build --release` pass.

## Sources

- OpenCode Go docs: https://dev.opencode.ai/docs/go/
- OpenCode CLI docs: https://opencode.ai/docs/cli/
- Current repo README: `README.md`
