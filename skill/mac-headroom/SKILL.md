---
name: mac-headroom
description: Diagnose macOS disk pressure with the mac-headroom CLI. Use when the user asks why their disk is full, what is using space, what grew, or whether caches are worth clearing.
---

# Using mac-headroom

mac-headroom measures. You interpret. Run it with `--json` and reason over the output.

## Order of operations

1. `mac-headroom --json diagnose` first, always. If `assessment` is `space_pinned`, stop
   hunting for files: the space is held by an APFS snapshot or purgeable space and
   no directory scan will find it. Tell the user to install the pending macOS update
   and reboot.
2. If `assessment` is `files_grew`, run `mac-headroom --json growth`. Report the entries
   with the largest positive `delta`. Look at what the directory actually is before
   suggesting deletion. A 30GB growth in a project folder may be a training run in
   progress, not garbage.
3. Only then consider `mac-headroom --json clean` (a dry run). Caches recover a few GB
   at most. Present what it found and the `why_safe` text from `mac-headroom --json list`
   so the user can decide.

## Rules

- Never pass `--yes` unless the user has explicitly said to delete, in this
  conversation, after seeing the dry run. Use `--only NAME` to limit it to what
  they approved.
- Never delete anything mac-headroom does not list. It has no cleaner for Chrome
  profile data, Docker volumes, or project directories on purpose.
- `bytes` figures are physical blocks. Trust them over `du -sh` or a tool's own
  "reclaimed" number.
- Every real deletion is appended to `~/.local/state/mac-headroom/audit.log`. Check it
  before assuming mac-headroom removed something.
- Setting `HEADROOM_NO_DELETE=1` makes `--yes` refuse. Suggest it for agent sessions
  where the user wants reports only.
