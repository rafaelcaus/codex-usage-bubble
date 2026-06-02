---
phase: 2
title: Refactor provider metadata and settings
status: completed
priority: P1
effort: 4h
dependencies:
  - 1
---

# Phase 2: Refactor Provider Metadata And Settings

## Overview

Remove hard-coded two-provider assumptions before adding OpenCode Go. Keep behavior identical for Claude Code and Codex.

## Requirements

- Functional: Represent enabled providers and positions by `ProviderId`, not one field per provider.
- Non-functional: Preserve old `settings.json` compatibility for `show_claude_code`, `show_codex`, and `bubble_positions.{claude,codex}`.

## Architecture

Introduce a small provider metadata layer near `usage::types` or `usage::registry`: stable ID, slug, display label key, tray icon ID, default enabled, and display mode. Existing providers use the current two-window mode. OpenCode Go uses a custom weekly/monthly four-bar mode.

Keep `ProviderId::ChatGpt` internally unless touching the code anyway. The user-facing label remains Codex. Avoid a mechanical rename that adds risk without feature value.

## Related Code Files

- Modify: `src/usage/types.rs`
- Modify: `src/usage/registry.rs`
- Modify: `src/settings.rs`
- Modify: `src/app.rs`
- Modify: `src/tray/mod.rs`
- Modify: `src/tray/badge.rs`
- Modify: `src/usage_color.rs`
- Modify: `src/bubble.rs`
- Modify: `src/panel.rs`

## Implementation Steps

1. Add `ProviderId::OpenCodeGo` and a stable `slug()` value `opencode-go`.
2. Add `ProviderId::all()` or equivalent ordered provider list: Claude, Codex, OpenCode Go.
3. Replace `Settings { show_claude_code, show_codex }` runtime logic with a provider-enabled map or compact struct that can hold `opencode_go`.
4. Preserve serde compatibility: deserialize old fields and write new fields only after migration, or keep old fields plus add `show_opencode_go` if map migration is too broad.
5. Extend `BubblePositions` to store OpenCode Go position while keeping old `claude` and `codex` JSON keys.
6. Update tray icon IDs and badge colors for third provider.
7. Add provider-specific display metadata: Claude/Codex mode=`two-window`; OpenCode Go mode=`weekly-monthly-four-bar`.
8. Update app loops to iterate providers instead of hand-writing Claude/Codex branches where this reduces match duplication.

## Success Criteria

- [ ] Existing `settings.json` with only Claude/Codex still loads.
- [ ] If all providers disabled, Claude Code is re-enabled as today.
- [ ] Three providers can have separate positions and tray icon IDs.
- [ ] OpenCode Go can select a custom four-bar renderer without changing Claude/Codex labels.
- [ ] No behavior change for existing Claude Code/Codex users.

## Risk Assessment

Settings migration is the highest regression risk. Mitigation: add serde/default tests with old two-provider JSON and new three-provider JSON.
