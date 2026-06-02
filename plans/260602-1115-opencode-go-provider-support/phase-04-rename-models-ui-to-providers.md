---
phase: 4
title: Rename Models UI to Providers
status: in-progress
priority: P2
effort: 2h
dependencies:
  - 2
---

# Phase 4: Rename Models UI To Providers

## Overview

Rename the menu and documentation wording from "Models" to "Providers" where the app is selecting quota sources.

Current result: partial. Provider wording and OpenCode Go listing are implemented. The OpenCode Go four-bar renderer is still blocked by Phase 1 because no stable quota source is available.

## Requirements

- Functional: Context menu uses "Providers" and lists Claude Code, Codex, OpenCode Go.
- Functional: OpenCode Go panel/bubble layout renders two grouped parts: Weekly on top and Monthly below, each with usage percent and remaining-time bars. Claude/Codex keep their current 5h/7d labels.
- Non-functional: Keep real model wording where it means actual LLM models, not provider selection.

## Architecture

This is mostly i18n and README copy. Keep internal `model` variable renames scoped to files touched by provider iteration. Do not churn every `model` local in renderer code just for style.

## Related Code Files

- Modify: `src/i18n/mod.rs`
- Modify: `src/i18n/locales/en.toml`
- Modify: `src/i18n/locales/ja.toml`
- Modify: `src/i18n/locales/ko.toml`
- Modify: `src/i18n/locales/vi.toml`
- Modify: `src/i18n/locales/zh-TW.toml`
- Modify: `src/app.rs`
- Modify: `README.md`

## Implementation Steps

1. Rename `LocaleStrings.models` to `providers`, or keep field name and change values if minimizing code churn is preferred.
2. Update English label to `Providers`; update other locales with best available translation.
3. Add `opencode_go_label = "OpenCode Go"` to all locale files.
4. Add localized generic labels for Weekly and Monthly group headers. Reuse existing usage-percent and remaining-time visual conventions.
5. Add or branch rendering for OpenCode Go's four-bar layout in `bubble.rs` and `panel.rs`; do not distort Claude/Codex two-bar layout.
6. Update README sections: "displayed models" -> "displayed providers"; "### Models" -> "### Providers".
7. Add locale schema tests for the new label.

## Success Criteria

- [x] Right-click menu says Providers.
- [x] README consistently describes selectable services as providers.
- [ ] OpenCode Go UI has Weekly and Monthly grouped sections, each with usage percent and remaining-time bars.
- [x] No accidental rename of OpenCode's documented `/models` command.
- [x] All embedded locale tests pass.

## Risk Assessment

Translations may be imperfect. Mitigation: keep provider product names untranslated and only translate the generic "Providers" label.
