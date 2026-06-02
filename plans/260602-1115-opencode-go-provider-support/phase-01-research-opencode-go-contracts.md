---
phase: 1
title: Research OpenCode Go contracts
status: blocked
priority: P1
effort: 2h
dependencies: []
---

# Phase 1: Research OpenCode Go Contracts

## Overview

Prove the OpenCode Go credential, provider ID, and usage-data contracts before implementation. This is the gate that prevents guessed percentages.

Current result: blocked. Provider/model IDs and console quota math are verified, but there is no verified stable CLI/public API for this app to fetch current weekly/monthly quota usage.

## Requirements

- Functional: Identify exact local credential path(s), JSON shape, auth key name, refresh/login command, weekly/monthly usage percent, and weekly/monthly reset time.
- Non-functional: Use primary sources first: official OpenCode docs, OpenCode source, locally installed `opencode` behavior if available.

## Architecture

OpenCode Go will only become a supported provider if it can return normalized weekly/monthly data from a stable source. Official docs confirm Go limits are 5h/weekly/monthly and dollar-value based, but not a public usage endpoint. For this app, OpenCode Go must render a custom four-bar layout: weekly usage percent + weekly remaining time in the upper section, monthly usage percent + monthly remaining time in the lower section.

## Related Code Files

- Read: `src/creds/mod.rs`
- Read: `src/creds/codex_auth.rs`
- Read: `src/usage/chatgpt.rs`
- Read: `src/usage/types.rs`
- Read: `src/usage/registry.rs`
- Created: `plans/260602-1115-opencode-go-provider-support/reports/research-opencode-go-contracts.md`

## Implementation Steps

1. Check official OpenCode Go docs for provider ID, limits, endpoints, and console usage semantics.
2. Check official OpenCode CLI/source for `auth.json` storage path and schema.
3. If OpenCode is installed locally, run safe read-only commands: `opencode stats --help`, `opencode providers --help`, `opencode models --help`.
4. Inspect local `~/.local/share/opencode/auth.json` only with user approval if privacy hook blocks or if file may contain API keys.
5. Search for an official usage endpoint or CLI command that returns current weekly/monthly usage percent or used/limit values plus reset/period end times.
6. Record exact request/response shape or command output needed by Phase 3, including how to compute remaining time for weekly and monthly sections.
7. If only console-authenticated usage exists, stop and ask user before planning implementation beyond auth detection.

## Success Criteria

- [x] Exact credential docs path documented.
- [x] Exact provider key documented: `opencode-go`.
- [x] Weekly/monthly usage percent and reset-time shape verified in console source.
- [x] Stable app-callable quota source confirmed unavailable from current CLI/docs.
- [ ] User decision recorded: console integration, limited detector, or defer.

## Risk Assessment

Main risk: OpenCode docs mention tracking usage in the console but do not document a public usage API. Mitigation: make Phase 3 blocked until a stable source exists; do not infer usage from price tables.
