# mac-headroom

Disk pressure diagnostician and cache cleaner for macOS developers.

Most cleaners race to a big "reclaimed" number. This one starts from the
lesson that caches only ever recover a few GB, and when a disk is actually
full the answer is usually one directory that grew, or space pinned by APFS
snapshots that `du` cannot see.

```
mac-headroom diagnose          # used/free/snapshots, and whether they moved together since last run
mac-headroom growth            # which directories grew since the last scan
mac-headroom list              # every cleaner and why it is safe
mac-headroom clean             # dry run: what would be deleted and how big it is
mac-headroom clean --yes       # actually delete
mac-headroom clean --only chrome-cache --only spotify-cache --yes
mac-headroom --json <any>      # machine-readable output

mac-headroom status            # one screen: disk, config, weekly job, recorded history
mac-headroom report            # the same as a self-contained HTML page, opened in your browser
mac-headroom config init       # write a commented example config
mac-headroom config check      # validate it and list what it defines

mac-headroom schedule install --weekday mon --hour 10 --only chrome-cache --only homebrew
mac-headroom schedule status   # installed? loaded? when? last run?
mac-headroom schedule run      # do the weekly routine now
mac-headroom schedule uninstall
```

## Principles

- Measure, report, delete only on explicit instruction. No heuristics about what "looks stale".
- Dry run by default. `--yes` is the only way anything is deleted, and `HEADROOM_NO_DELETE=1` disables even that.
- Every cleaner states why it is safe.
- Cleaners that touch a running app's cache are skipped while that app is open.
- Sizes are physical blocks, not logical length, so sparse files and clones are not overcounted.
- Chrome's profile data (Application Support) is never touched. Only its cache is.
- Every real deletion is appended to `~/.local/state/mac-headroom/audit.log`.

## Diagnose before you delete

`diagnose` records used and free space on each run. If free space fell by
much more than used space grew, the difference is not files. It is almost
always a staged macOS update snapshot, and the fix is to install it and reboot.
This exact failure mode hid 14GB for months before this tool existed.

`growth` walks a root once, records the size of every path to a given depth,
and diffs against the previous scan. New and deleted paths are included. It
defaults to your home directory, which is usually about two thirds of the data
volume; pass a path to scan `/Library`, `/Applications` or anywhere else.

## Your own cleaners

`~/.config/mac-headroom/config.toml` adds cleaners with the same fields the
built-ins have, plus two filters for the leaks that need judgement:

```toml
[[cleaner]]
name = "myapp-model-cache"
summary = "Stale compiled model bundles from MyApp dev builds"
why_safe = "Only the newest bundle per arch is used. Older siblings are leftovers."
paths = ["~/Library/Caches/com.example.myapp/models/*/*"]
keep_newest = 1              # per directory, keep the N most recently modified matches
skip_if_running = "MyApp"

[[cleaner]]
name = "agent-job-scratch"
summary = "Scratch dirs left by finished background jobs"
why_safe = "Untouched for a week means the job is long done."
paths = ["~/.claude/jobs/*/tmp"]
older_than_days = 7          # only when nothing inside was modified for N days

disable = ["spotify-cache"]  # built-ins to leave out of a plain `clean`
```

Paths must start with `~/` and name a directory under home plus something
inside it, so `~/Library` or `~/*` are refused. `why_safe` is mandatory. A
config with any problem stops every command rather than falling back to the
built-ins, because a half-understood config is how the wrong thing gets deleted.
Kept matches are shown in the output with the reason they were kept.

## Weekly job

`schedule install` writes a launchd LaunchAgent that runs `mac-headroom schedule run`
at the given time each week (or on next wake if the Mac was asleep). Each run does
`diagnose`, a home `growth` scan, and `clean --yes` for the cleaners you named at
install time, and appends everything to `~/Library/Logs/mac-headroom.log`. Because
`diagnose` and `growth` record history on every run, the weekly job is what makes
"what changed since last week" answerable.

Install the binary somewhere stable first (`cargo install --path .`); the plist
points at the binary's absolute path.

## Report

`report` writes one self-contained HTML file (no server, no network, no
external assets) and opens it: disk composition, used and free over time,
the last growth diff as bars, the weekly job, every cleaner with its reason,
and the deletion audit. It is read-only and built from saved state, so it
never scans or deletes. Use `--no-open` to just write the file, `--out` to
choose where.

## With an AI agent

mac-headroom is designed to be run by a coding agent and interpreted by it. The
`--json` flag gives structured output for every subcommand. `skill/mac-headroom/SKILL.md`
is a drop-in skill for Claude Code (copy it to `~/.claude/skills/mac-headroom/`) that
tells the agent the order of operations and the deletion rules.

## Install

```
cargo install --path .
```

## Status

Working proof of concept. Not yet: Homebrew packaging.
