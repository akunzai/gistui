# Where a finished action lands stays with each apply handler

Opening a screen after async work is recorded at intent time: `DeferredEntry` snapshots the return screen when the key is pressed and moves through the request to the apply (`docs/agents/architecture.md`, Screen state machine). Leaving on success has no such value. Each success handler says where the user lands, by popping the screens it knows sit between it and its target. The 2026-09 architecture review raised the asymmetry (candidate 4) after #520, #475, #476 and 9802222 had each fixed a landing. The proposal was to record "where success lands" as plain data on each request at key time, and have one generic handler return there.

We decided **not** to. By the end of the 2026-09 sync work (#524–#526) every landing rule had one named, tested home:

- `bg::land_after_confirmed_sync` for a download and a push: leave Confirm if on it, then a Diff the write made stale.
- `AppState::cancel_confirm_after_delete` for a gist delete: pop the deleted gist's own `GistDetail`.
- `cancel_confirm` for compaction.
- `back_to_list` for remove-file and create. Both are launched only from the List, which architecture.md's `back_to_list` rule requires.
- The restore apply's return to Revisions.
- Nothing for description, star, and fork, which change no screen.

The deletion test says a declared landing would move this logic, not concentrate it. Deciding which now-stale screens to skip still happens somewhere. It would move into every request as a field, plus a generic handler that interprets it. No landing bug has been traced since #520.

**Revisit if**: a landing bug is traced to one of these paths. Fix that path's rule in its own function, the way #520 did, and do not introduce a request-wide landing value. Reconsider the value only if the same kind of landing bug recurs across several handlers after this ADR.
