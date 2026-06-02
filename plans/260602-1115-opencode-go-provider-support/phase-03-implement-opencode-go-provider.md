---
phase: 3
title: "Implement OpenCode Go provider"
status: pending
priority: P1
effort: "5h"
dependencies: [1, 2]
---

# Phase 3: Implement OpenCode Go Provider

## Overview

Add OpenCode Go credential discovery and polling once Phase 1 proves the source of truth.

## Requirements

- Functional: When enabled and authenticated, OpenCode Go returns weekly/monthly usage percent and reset time, enough for four bars: usage percent + remaining time for each section.
- Non-functional: Do not store or transmit API keys outside OpenCode/OpenCode Go endpoints. Do not mutate OpenCode auth files.

## Architecture

Create an OpenCode Go credential source that reads OpenCode's auth store and a provider implementation that maps verified weekly/monthly usage percent + reset time into a provider-specific snapshot. Do not force OpenCode Go into the existing `UsageWindows.primary/secondary` shape if that would lose the grouped weekly/monthly rendering semantics.

## Related Code Files

- Create: `src/creds/opencode_auth.rs`
- Create: `src/usage/opencode_go.rs`
- Modify: `src/creds/mod.rs`
- Modify: `src/usage/mod.rs`
- Modify: `src/usage/registry.rs`
- Modify: `src/usage/refresh.rs`
- Modify: `src/app.rs`

## Implementation Steps

1. Add `LocalOpenCodeGoCreds` with path detection from Phase 1. Expected primary path: `%LOCALAPPDATA%`/XDG-equivalent for `opencode/auth.json`; verify exact Windows path before coding.
2. Parse only the OpenCode Go auth entry. Support API-key style auth if Phase 1 confirms schema.
3. Add `RefreshHint::LocalOpenCodeCli` and spawn `opencode auth login` or equivalent only if Phase 1 confirms the command.
4. Add `OpenCodeGoProvider::poll`, using the verified usage endpoint/command.
5. Add a provider-specific snapshot type if needed, for example `ProviderUsage::TwoWindow(UsageWindows)` and `ProviderUsage::WeeklyMonthly { weekly: Window, monthly: Window }`.
6. Map returned weekly/monthly source data to usage percent and reset-time windows. Existing renderer can then draw usage and remaining-time bars for each window.
7. Return `AuthRequired` on 401/403 or missing provider auth.
8. Add unit tests for credential parsing and response-to-snapshot mapping.

## Success Criteria

- [ ] Provider compiles behind the existing registry.
- [ ] Missing OpenCode auth produces `NoCredentials`, not panic.
- [ ] Expired/invalid auth produces `AuthRequired`.
- [ ] Weekly/monthly usage percent clamps to 0-100 and remaining-time bars derive from verified reset times.
- [ ] If Phase 1 cannot verify usage, this phase is not implemented.

## Risk Assessment

OpenCode Go limits are dollar-value based. If the usage source returns dollars instead of percentages, compute percent only as `used / limit * 100` from server-provided weekly/monthly values. Never estimate usage from model prices. If reset times are unavailable, stop and ask before shipping remaining-time bars with guessed periods.
