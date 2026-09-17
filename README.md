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
mac-headroom volumes           # what else shares this disk, and what a scan cannot reach
mac-headroom check             # is free space low? notify and write a report if so
```

## Principles

- Measure, report, delete only on explicit instruction. No heuristics about what "looks stale".
- Dry run by default. `--yes` is the only way anything is deleted, and `HEADROOM_NO_DELETE=1` disables even that.
- Every cleaner states why it is safe.
- Cleaners that touch a running app's cache are skipped while that app is open.
- Sizes are physical blocks, not logical length, so sparse files and clones are not overcounted.
- Chrome's profile data (Application Support) is never touched. Only its cache is.
- Every real deletion, and only a real deletion, is appended to
  `~/.local/state/mac-headroom/audit.log`. A target a filter kept is not
  recorded, because an audit trail that overstates is worse than none.

## Diagnose before you delete

`diagnose` records used and free space on each run. If free space fell by
much more than used space grew, the difference is not files. It is almost
always a staged macOS update snapshot, and the fix is to install it and reboot.
This exact failure mode hid 14GB for months before this tool existed.

`growth` walks a root once, records the size of every path to a given depth,
and diffs against the previous scan. New and deleted paths are included. It
defaults to your home directory, which is usually about two thirds of the data
volume. Name extra directories in the config to cover the rest:

```toml
scan_roots = ["/Applications", "/Library", "/opt"]
```

Those are scanned alongside home by `growth`, by the weekly job, and in the
report, where they nest under one total so nothing is double counted. A scan
never crosses into another filesystem, so a mounted disk image is not counted
twice. Without sudo a system directory skips what it cannot read, and the
count of skipped entries is reported rather than quietly folded in.

## Files an app has lost track of

Some apps keep their media on disk and their record of it in a database. Replace
the database and the files are stranded: nothing points at them, and the app's
own storage screen never offers to remove them, because it reports what the
database knows. WhatsApp does this whenever you re-link a Mac as a device. On
the machine this was written on, that left 16GB across 55,607 files while the
app reported a few hundred megabytes.

```
mac-headroom clean --only whatsapp-orphans          # dry run
mac-headroom clean --only whatsapp-orphans --yes
```

It reads WhatsApp's database, including the write-ahead log, and removes only
files nothing in it references. It is skipped while WhatsApp is open, refuses if
the database cannot be read, and refuses if the database comes back empty, since
a failed read and a genuinely empty one look identical and the wrong answer
deletes everything.

This is deliberately app-specific. There is no general way to tell which of an
application's files it has stopped caring about, and a heuristic would
eventually delete something real. Because it removes what you would recognise as
your own photos and videos, it never runs as part of a plain `clean`: you have
to name it, or list it under `enable` in the config.

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
external assets) and opens it, under three tabs: **Space** (the verdict, disk
composition, the other volumes sharing the disk, space over time, and the
breakdown of where it has gone), **Cleaning** (the weekly job and every cleaner
with its reason), and **Deletions** (the audit trail). Each tab has its own
URL fragment, so a link can point at one. It is read-only and built from saved state, so it
never scans or deletes. Use `--no-open` to just write the file, `--out` to
choose where.

## What a scan cannot see

A directory walk only ever covers one filesystem, so on a Mac it misses a
surprising amount: the sealed system volume, swap, Preboot and Recovery all
share the same APFS container and draw on the same free space. `volumes` names
them with what each has consumed, which is what the "system and purgeable"
figure is actually made of.

It also lists attached disk images with the file each is read from, since a
mounted image looks like a volume but the space belongs to that file. An iOS
simulator runtime, for instance, mounts as an 18GB volume while costing a 7.8GB
file, and that file lives under `/System/Library/AssetsV2`, which looks like
part of the system but is firmlinked onto the data volume.

## Noticing before it is too late

`schedule install` also sets up an hourly watch. It is one `diskutil` call and
no directory walk, so it costs nothing to run often. When free space falls
below `alert_below_percent` of the disk (5 by default, 0 turns it off) it
writes a fresh report and posts a notification naming it, then stays quiet for
twelve hours so one full disk is one notification rather than one an hour.

```
mac-headroom check           # run that same test now
mac-headroom check --force   # ignore the cooldown
mac-headroom schedule install --no-watch
```

The weekly job also leaves a current report behind when it finishes, so there
is always one to open.

## With an AI agent

mac-headroom is built to be run by a coding agent and interpreted by it. A
full disk is a diagnosis problem, and the reasoning is the part an agent is
good at; what it lacks is trustworthy numbers and a tool that refuses to do
anything reckless. So the split is deliberate: this measures and explains,
the agent decides.

- `--json` on every subcommand, with stable cleaner names to refer to.
- Every cleaner carries a `why_safe` line the agent can quote back to you.
- Read-only by default. `--yes` is the only way anything is deleted, and
  `HEADROOM_NO_DELETE=1` disables even that, so the tool can be handed to an
  agent knowing the worst case is a report.
- Physical block sizes, snapshot detection and scan history are things an
  agent cannot work out for itself between sessions. `skill/mac-headroom/SKILL.md`
is a drop-in skill for Claude Code (copy it to `~/.claude/skills/mac-headroom/`) that
tells the agent the order of operations and the deletion rules.

## Install

```
cargo install --git https://github.com/joshua-mason/mac-headroom --tag v0.2.0
```

Or from a clone, `cargo install --path .`. Either way the binary lands in
`~/.cargo/bin`, which is where the weekly job will point at it, so install it
somewhere stable before running `schedule install`.

Then, in order:

```
mac-headroom diagnose          # is anything hiding?
mac-headroom growth            # a baseline of where the space is
mac-headroom config init       # optional: your own cleaners and extra scan roots
mac-headroom clean             # dry run, read what it would remove and why
mac-headroom schedule install  # weekly clean, plus the hourly low space watch
mac-headroom report            # the whole picture in a browser
```

## Status

0.2.0. Adds `whatsapp-orphans`, the first cleaner that finds files by asking an
application what it still references rather than by matching a path. Working on macOS 15 and 26 on Apple silicon.
Not yet: Homebrew packaging, or prebuilt binaries.
