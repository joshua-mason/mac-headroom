---
name: mac-headroom
description: Diagnose macOS disk pressure with the mac-headroom CLI. Use when the user asks why their disk is full, what is using space, what grew, or whether caches are worth clearing.
---

# Using mac-headroom

mac-headroom measures. You interpret. Run it with `--json` and reason over the output.

## First, the lay of the land

`mac-headroom --json status` is read-only and shows the disk, the config file and
its problems, which cleaners are enabled, whether the weekly job is installed and
when it last ran, and how much history exists. Start here when the user asks "what
is set up?" or "what does this do on my machine?".

When the user wants to *see* the picture rather than read it, `mac-headroom report`
writes a self-contained HTML page and opens it in their browser. It is read-only.

## Order of operations

1. `mac-headroom --json diagnose` first, always. If `assessment` is `space_pinned`, stop
   hunting for files: the space is held by an APFS snapshot or purgeable space and
   no directory scan will find it. Tell the user to install the pending macOS update
   and reboot.
2. If `assessment` is `files_grew`, run `mac-headroom --json growth`. Note it scans the
   home directory only. On most machines that is roughly two thirds of the data volume;
   the rest is `/Applications`, `/Library`, `/opt/homebrew` and `/private/var`. Adding
   those to `scan_roots` in the config makes every scan and the report cover them; a
   one-off is `mac-headroom --json growth /Library`. A non-zero `skipped` on a report
   means the figure is a floor, because the scan could not read everything without sudo. Report the entries
   with the largest positive `delta`. Look at what the directory actually is before
   suggesting deletion. A 30GB growth in a project folder may be a training run in
   progress, not garbage.
3. Only then consider `mac-headroom --json clean` (a dry run). Caches recover a few GB
   at most. Present what it found and the `why_safe` text from `mac-headroom --json list`
   so the user can decide.

## Adding a cleaner for the user

When a growth report or a manual hunt finds a recurring leak, the durable fix is a
cleaner in `~/.config/mac-headroom/config.toml` (see `mac-headroom config init` for
the format). Write the `why_safe` line to be honest about what is lost. Use
`keep_newest` for build caches that leak one copy per build, and `older_than_days`
for scratch that is safe once a job has finished. Then run `mac-headroom config check`
and a dry run of `clean --only <name>`, and show the user what would be deleted and
what was kept before suggesting `--yes`.

## Alerts

`mac-headroom --json check` is the cheap low-space test the hourly watch job runs:
one `diskutil` call, no scan. `low` says whether free space is under the user's
threshold, and it writes a report and notifies only when it is. Run it freely; it
deletes nothing. `schedule status` shows whether the watch is installed and at what
threshold.

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
- `mac-headroom --json schedule status` shows whether the weekly job is installed and
  when it last ran. `~/Library/Logs/mac-headroom.log` has each weekly run's diagnose,
  growth and clean output, which is the best history of what changed on this machine.
- `schedule install` and `schedule run` both delete (the cleaners named with `--only`).
  Treat them like `--yes`: only on explicit instruction.
- Setting `HEADROOM_NO_DELETE=1` makes `--yes` refuse. Suggest it for agent sessions
  where the user wants reports only.
