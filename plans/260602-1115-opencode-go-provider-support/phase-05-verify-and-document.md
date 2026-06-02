---
phase: 5
title: "Verify and document"
status: pending
priority: P1
effort: "3h"
dependencies: [3, 4]
---

# Phase 5: Verify And Document

## Overview

Prove the third-provider workflow works and document setup/privacy accurately.

## Requirements

- Functional: Claude Code, Codex, and OpenCode Go can be toggled independently; polling and tray refresh do not regress.
- Non-functional: Privacy docs must list every local file read and network endpoint called.

## Architecture

Verification must cover pure tests, build, and Windows manual smoke tests. OpenCode Go cannot be fully claimed without a real authenticated account or a captured fixture from Phase 1.

## Related Code Files

- Modify: `README.md`
- Optional Modify: `docs/release-process.md` only if release steps change
- Read: `src/app.rs`
- Read: `src/settings.rs`
- Read: `src/usage/*`
- Read: `src/creds/*`

## Implementation Steps

1. Run `cargo test`.
2. Run `cargo build --release`.
3. Run app with old settings file and confirm migration/defaults.
4. Run app with all three providers enabled; verify separate bubbles, positions, tray icons, panel labels, and menu toggles.
5. Verify missing OpenCode Go auth shows auth/no-credentials state without breaking other providers.
6. With real OpenCode Go auth, verify weekly/monthly percent and remaining-time bars match the official console/source found in Phase 1.
7. Update README privacy section with OpenCode auth file and endpoints.

## Success Criteria

- [ ] Tests pass.
- [ ] Release build passes.
- [ ] Existing two-provider settings migrate.
- [ ] Three-provider UI works on Windows.
- [ ] OpenCode Go renders four bars: weekly usage percent, weekly remaining time, monthly usage percent, monthly remaining time.
- [ ] README setup/privacy docs are accurate.
- [ ] No unresolved OpenCode Go usage-source uncertainty remains.

## Risk Assessment

Manual verification needs real OpenCode Go auth. If unavailable, ship code only behind disabled-by-default toggle and explicitly mark e2e verification deferred.
