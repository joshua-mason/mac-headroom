//! A single self-contained HTML file with everything `status`, `diagnose`
//! history, the last `growth` diff, the cleaner table and the audit trail
//! show, drawn as charts and tables. Read-only: it reads saved state and
//! never scans or deletes. No server, no network, no external assets.

use crate::util::{home, now, state_dir, stdout_of};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct Reading {
    at: u64,
    used: u64,
    free: u64,
}

#[derive(Serialize)]
struct AuditEntry {
    at: u64,
    cleaner: String,
    result: String,
    bytes: u64,
    path: String,
}

#[derive(Serialize)]
pub struct Data {
    generated_at: u64,
    host: String,
    version: &'static str,
    home: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk: Option<DiskNow>,
    history: Vec<Reading>,
    #[serde(skip_serializing_if = "Option::is_none")]
    growth: Option<crate::growth::Report>,
    config: crate::config::Check,
    cleaners: Vec<crate::cleaners::Cleaner>,
    schedule: crate::schedule::Status,
    audit: Vec<AuditEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_run_log: Option<String>,
}

#[derive(Serialize)]
struct DiskNow {
    used: u64,
    free: u64,
    total: u64,
    other: u64,
}

fn history() -> Vec<Reading> {
    fs::read_to_string(state_dir().join("history.tsv"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some(Reading {
                at: f.next()?.parse().ok()?,
                used: f.next()?.parse().ok()?,
                free: f.next()?.parse().ok()?,
            })
        })
        .collect()
}

fn audit(limit: usize) -> Vec<AuditEntry> {
    let text = fs::read_to_string(state_dir().join("audit.log")).unwrap_or_default();
    let mut entries: Vec<AuditEntry> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(5, '\t').collect();
            if f.len() < 5 {
                return None;
            }
            Some(AuditEntry {
                at: f[0].parse().ok()?,
                cleaner: f[1].to_string(),
                result: f[2].to_string(),
                bytes: f[3].parse().unwrap_or(0),
                path: f[4].to_string(),
            })
        })
        .collect();
    entries.reverse();
    entries.truncate(limit);
    entries
}

fn last_run_log() -> Option<String> {
    let text = fs::read_to_string(crate::schedule::log_path()).ok()?;
    let start = text.rfind("=== mac-headroom scheduled run ===")?;
    let line_start = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    Some(text[line_start..].trim_end().to_string())
}

fn hostname() -> String {
    let name = stdout_of("scutil", &["--get", "ComputerName"]);
    let name = name.trim();
    if name.is_empty() {
        stdout_of("hostname", &["-s"]).trim().to_string()
    } else {
        name.to_string()
    }
}

pub fn gather() -> Data {
    let cleaners = crate::config::all_cleaners().unwrap_or_default();
    Data {
        generated_at: now(),
        host: hostname(),
        version: env!("CARGO_PKG_VERSION"),
        home: home().to_string_lossy().into_owned(),
        disk: crate::diag::disk().map(|d| DiskNow {
            used: d.used,
            free: d.free,
            total: d.total,
            other: d.total.saturating_sub(d.used + d.free),
        }),
        history: history(),
        growth: crate::growth::from_saved(&home(), 3, 100 << 20, 15),
        config: crate::config::check(),
        cleaners,
        schedule: crate::schedule::status(),
        audit: audit(25),
        last_run_log: last_run_log(),
    }
}

pub fn default_path() -> PathBuf {
    state_dir().join("report.html")
}

pub fn write(data: &Data, out: &Path) -> std::io::Result<()> {
    // `</` inside a <script> would end it early; JSON never needs the raw form.
    let json = serde_json::to_string(data)
        .expect("serialize report")
        .replace("</", "<\\/");
    fs::write(out, TEMPLATE.replace("__DATA__", &json))
}

pub fn open(path: &Path) -> bool {
    std::process::Command::new("open")
        .arg(path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const TEMPLATE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>mac-headroom report</title>
<style>
:root{color-scheme:light;
  --page:#f9f9f7;--surface:#fcfcfb;--ink:#0b0b0b;--ink2:#52514e;--muted:#898781;
  --grid:#e1e0d9;--axis:#c3c2b7;--border:rgba(11,11,11,.10);
  --s1:#2a78d6;--s2:#eb6834;
  --meter-used:#2a78d6;--meter-other:#9ec5f4;--meter-track:#cde2fb;
  --grow:#e34948;--shrink:#2a78d6;
  --good:#0ca30c;--goodtext:#006300;--critical:#d03b3b}
@media (prefers-color-scheme:dark){:root{color-scheme:dark;
  --page:#0d0d0d;--surface:#1a1a19;--ink:#fff;--ink2:#c3c2b7;--muted:#898781;
  --grid:#2c2c2a;--axis:#383835;--border:rgba(255,255,255,.10);
  --s1:#3987e5;--s2:#d95926;
  --meter-used:#3987e5;--meter-other:#1c5cab;--meter-track:#104281;
  --grow:#e66767;--shrink:#3987e5;
  --goodtext:#0ca30c}}
*{box-sizing:border-box}
body{margin:0;background:var(--page);color:var(--ink);font:14px/1.45 system-ui,-apple-system,"Segoe UI",sans-serif;padding:24px 16px 48px}
main{max-width:960px;margin:0 auto}
h1{font-size:22px;font-weight:600;margin:0 0 4px}
h2{font-size:15px;font-weight:600;margin:0 0 12px}
.sub{color:var(--ink2);margin:0 0 24px}
.card{background:var(--surface);border:1px solid var(--border);border-radius:8px;padding:20px;margin-bottom:16px}
.row{display:flex;gap:16px;flex-wrap:wrap}
.tile{flex:1 1 160px}
.tile .label{color:var(--ink2);font-size:13px}
.tile .value{font-size:28px;font-weight:600;line-height:1.2}
.hero{font-size:48px;font-weight:600;line-height:1.1}
.muted{color:var(--muted)}
.ink2{color:var(--ink2)}
.meter{display:flex;height:16px;border-radius:4px;overflow:hidden;background:var(--meter-track);margin:14px 0 8px}
.meter>div{height:100%;border-right:2px solid var(--surface)}
.legend{display:flex;gap:16px;flex-wrap:wrap;font-size:13px;color:var(--ink2)}
.sw{display:inline-block;width:10px;height:10px;border-radius:2px;vertical-align:-1px;margin-right:6px}
.key{display:inline-block;width:14px;height:2px;vertical-align:3px;margin-right:6px;border-radius:1px}
svg{display:block;width:100%;height:auto}
.axis text{fill:var(--muted);font-size:11px;font-variant-numeric:tabular-nums}
.grid line{stroke:var(--grid);stroke-width:1}
.base{stroke:var(--axis);stroke-width:1}
.line{fill:none;stroke-width:2;stroke-linejoin:round;stroke-linecap:round}
.dot{stroke:var(--surface);stroke-width:2}
.endlabel{fill:var(--ink2);font-size:12px}
.cross{stroke:var(--axis);stroke-width:1;display:none}
#tip{position:fixed;pointer-events:none;background:var(--surface);color:var(--ink);border:1px solid var(--border);border-radius:6px;padding:8px 10px;font-size:12px;box-shadow:0 4px 16px rgba(0,0,0,.12);display:none;max-width:420px;word-break:break-all;z-index:2}
#tip b{font-weight:600}
.bars{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr) 84px;gap:6px 10px;align-items:center;font-size:13px}
.bars .path{font-variant-numeric:tabular-nums;direction:rtl;text-align:left;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--ink2)}
.bars .path span{direction:ltr;unicode-bidi:isolate}
.bars .val{text-align:right;font-variant-numeric:tabular-nums}
.track{position:relative;height:16px}
.track .bar{position:absolute;top:0;height:16px}
.track .mid{position:absolute;top:0;bottom:0;left:50%;width:1px;background:var(--axis)}
.bar.pos{border-radius:0 4px 4px 0;background:var(--grow)}
.bar.neg{border-radius:4px 0 0 4px;background:var(--shrink)}
.bar.size{border-radius:0 4px 4px 0;background:var(--s1)}
table{border-collapse:collapse;width:100%;font-size:13px}
th{text-align:left;color:var(--ink2);font-weight:600;padding:6px 8px;border-bottom:1px solid var(--axis);white-space:nowrap}
td{padding:6px 8px;border-bottom:1px solid var(--grid);vertical-align:top}
td.num{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}
td.name{white-space:nowrap}
td.mono,code,pre{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px}
pre{background:var(--page);border:1px solid var(--border);border-radius:6px;padding:12px;overflow-x:auto;margin:0;white-space:pre-wrap}
.pill{display:inline-block;padding:1px 8px;border-radius:999px;font-size:12px;border:1px solid var(--border);color:var(--ink2)}
.pill.on{color:var(--goodtext);border-color:var(--good)}
.pill.warn{color:var(--critical);border-color:var(--critical)}
details{margin-top:12px}
summary{cursor:pointer;color:var(--ink2);font-size:13px}
.empty{color:var(--ink2);padding:8px 0}
.overflow{overflow-x:auto}
</style>
</head>
<body>
<main>
<h1>mac-headroom report</h1>
<p class="sub" id="sub"></p>
<div id="tip"></div>

<section class="card" id="disk"></section>
<section class="card" id="history"></section>
<section class="card" id="growth"></section>
<section class="card" id="job"></section>
<section class="card" id="cleaners"></section>
<section class="card" id="audit"></section>
</main>
<script id="data" type="application/json">__DATA__</script>
<script>
const D = JSON.parse(document.getElementById('data').textContent);
const U = ['B','KB','MB','GB','TB'];
const h = b => { let v = Math.abs(b), i = 0; while (v >= 1024 && i < 4) { v /= 1024; i++; } return (i ? v.toFixed(1) : v) + ' ' + U[i]; };
const sg = d => (d < 0 ? '−' : '+') + h(d);
const when = t => new Date(t * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
const ago = t => { const s = Math.max(0, D.generated_at - t); const d = Math.floor(s / 86400), hh = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60); return d ? `${d}d ${hh}h ago` : hh ? `${hh}h ${m}m ago` : `${m}m ago`; };
const tilde = p => p.startsWith(D.home) ? '~' + p.slice(D.home.length) : p;
const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const $ = id => document.getElementById(id);
const tip = $('tip');
function showTip(html, x, y) { tip.innerHTML = html; tip.style.display = 'block'; const w = tip.offsetWidth, hh = tip.offsetHeight; tip.style.left = Math.min(x + 14, innerWidth - w - 8) + 'px'; tip.style.top = (y + 14 + hh > innerHeight ? y - hh - 10 : y + 14) + 'px'; }
function hideTip() { tip.style.display = 'none'; }

$('sub').textContent = `${D.host} · generated ${when(D.generated_at)} · mac-headroom ${D.version}`;

/* ---- Disk ---- */
(() => {
  const d = D.disk, el = $('disk');
  if (!d) { el.innerHTML = '<h2>Disk</h2><p class="empty">diskutil was unavailable.</p>'; return; }
  const pct = x => (100 * x / d.total).toFixed(1) + '%';
  el.innerHTML = `<h2>Disk</h2>
    <div class="row">
      <div class="tile"><div class="label">Free</div><div class="hero">${h(d.free)}</div></div>
      <div class="tile"><div class="label">Used by your data</div><div class="value">${h(d.used)}</div><div class="muted">${pct(d.used)}</div></div>
      <div class="tile"><div class="label">System, VM, purgeable</div><div class="value">${h(d.other)}</div><div class="muted">${pct(d.other)}</div></div>
      <div class="tile"><div class="label">Total</div><div class="value">${h(d.total)}</div></div>
    </div>
    <div class="meter" role="img" aria-label="Disk composition">
      <div style="width:${100 * d.used / d.total}%;background:var(--meter-used)" data-tip="<b>Used by your data</b><br>${h(d.used)} (${pct(d.used)})"></div>
      <div style="width:${100 * d.other / d.total}%;background:var(--meter-other)" data-tip="<b>System, VM, purgeable</b><br>${h(d.other)} (${pct(d.other)})"></div>
    </div>
    <div class="legend"><span><i class="sw" style="background:var(--meter-used)"></i>used by your data</span><span><i class="sw" style="background:var(--meter-other)"></i>system, VM, purgeable</span><span><i class="sw" style="background:var(--meter-track)"></i>free</span></div>`;
})();

/* ---- History line chart ---- */
(() => {
  const el = $('history'), H = D.history;
  if (H.length < 2) { el.innerHTML = `<h2>Used and free over time</h2><p class="empty">${H.length ? 'One reading so far.' : 'No readings yet.'} Each <code>diagnose</code> or weekly run adds one. When free falls faster than used grows, the gap is snapshot-pinned space, not files.</p>`; return; }
  const W = 860, HT = 260, L = 64, R = 70, T = 16, B = 34;
  const xs = H.map(r => r.at), x0 = Math.min(...xs), x1 = Math.max(...xs);
  const ymax = Math.max(...H.map(r => Math.max(r.used, r.free))) * 1.05;
  const X = t => L + (x1 === x0 ? (W - L - R) / 2 : (t - x0) / (x1 - x0) * (W - L - R));
  const Y = v => T + (HT - T - B) * (1 - v / ymax);
  const GB = 1024 ** 3, step = ymax / GB > 200 ? 50 : ymax / GB > 80 ? 25 : ymax / GB > 30 ? 10 : 5;
  let grid = '', ticks = '';
  for (let g = 0; g * GB <= ymax; g += step) { grid += `<line x1="${L}" x2="${W - R}" y1="${Y(g * GB)}" y2="${Y(g * GB)}"/>`; ticks += `<text x="${L - 8}" y="${Y(g * GB) + 4}" text-anchor="end">${g} GB</text>`; }
  const path = k => H.map((r, i) => (i ? 'L' : 'M') + X(r.at).toFixed(1) + ' ' + Y(r[k]).toFixed(1)).join(' ');
  const last = H[H.length - 1], first = H[0];
  const xt = [first, last].map(r => `<text x="${X(r.at)}" y="${HT - 10}" text-anchor="${r === first ? 'start' : 'end'}">${when(r.at)}</text>`).join('');
  el.innerHTML = `<h2>Used and free over time</h2>
    <div class="legend" style="margin-bottom:8px"><span><i class="key" style="background:var(--s1)"></i>used by your data</span><span><i class="key" style="background:var(--s2)"></i>free</span></div>
    <svg viewBox="0 0 ${W} ${HT}" id="hsvg">
      <g class="grid">${grid}</g>
      <g class="axis">${ticks}${xt}</g>
      <line class="base" x1="${L}" x2="${W - R}" y1="${Y(0)}" y2="${Y(0)}"/>
      <line class="cross" id="cross" y1="${T}" y2="${Y(0)}"/>
      <path class="line" d="${path('used')}" stroke="var(--s1)"/>
      <path class="line" d="${path('free')}" stroke="var(--s2)"/>
      <circle class="dot" cx="${X(last.at)}" cy="${Y(last.used)}" r="4" fill="var(--s1)"/>
      <circle class="dot" cx="${X(last.at)}" cy="${Y(last.free)}" r="4" fill="var(--s2)"/>
      <text class="endlabel" x="${X(last.at) + 8}" y="${Y(last.used) + 4}">${h(last.used)}</text>
      <text class="endlabel" x="${X(last.at) + 8}" y="${Y(last.free) + 4}">${h(last.free)}</text>
    </svg>
    <details><summary>Table of readings</summary><div class="overflow"><table><tr><th>When</th><th>Used</th><th>Free</th></tr>${H.map(r => `<tr><td>${when(r.at)}</td><td class="num">${h(r.used)}</td><td class="num">${h(r.free)}</td></tr>`).join('')}</table></div></details>`;
  const svg = $('hsvg'), cross = $('cross');
  svg.addEventListener('mousemove', e => {
    const pt = svg.createSVGPoint(); pt.x = e.clientX; pt.y = e.clientY;
    const p = pt.matrixTransform(svg.getScreenCTM().inverse());
    let best = H[0], bd = Infinity; for (const r of H) { const d = Math.abs(X(r.at) - p.x); if (d < bd) { bd = d; best = r; } }
    cross.setAttribute('x1', X(best.at)); cross.setAttribute('x2', X(best.at)); cross.style.display = 'block';
    showTip(`<b>${when(best.at)}</b><br>used ${h(best.used)}<br>free ${h(best.free)}`, e.clientX, e.clientY);
  });
  svg.addEventListener('mouseleave', () => { cross.style.display = 'none'; hideTip(); });
})();

/* ---- Growth ---- */
(() => {
  const el = $('growth'), g = D.growth;
  if (!g) { el.innerHTML = '<h2>What grew</h2><p class="empty">No growth scan saved yet. Run <code>mac-headroom growth</code> once for a baseline, and again later for the diff.</p>'; return; }
  const diff = g.previous_at != null;
  const head = diff
    ? `Compared with the scan from ${ago(g.previous_at)}: <b>${sg(g.total - g.previous_total)}</b> overall (${h(g.previous_total)} → ${h(g.total)}). Changes of ${h(g.min_bytes)} or more, largest first.`
    : `Baseline scan from ${ago(g.scanned_at)}: ${h(g.total)} under ~ (depth ${g.depth}). Largest entries. Run <code>mac-headroom growth</code> again later to see what changed.`;
  if (!g.entries.length) { el.innerHTML = `<h2>What grew</h2><p class="ink2">${head}</p><p class="empty">Nothing changed by that much.</p>`; return; }
  const max = Math.max(...g.entries.map(e => Math.abs(diff ? e.delta : e.bytes)));
  const rows = g.entries.map(e => {
    const v = diff ? e.delta : e.bytes, w = 50 * Math.abs(v) / max;
    const bar = diff
      ? (v >= 0 ? `<div class="bar pos" style="left:50%;width:${w}%"></div>` : `<div class="bar neg" style="right:50%;width:${w}%"></div>`)
      : `<div class="bar size" style="left:0;width:${2 * w}%"></div>`;
    const note = diff ? (e.previous == null ? ' <span class="pill">new</span>' : e.bytes === 0 ? ' <span class="pill">gone</span>' : '') : '';
    const tipHtml = `<b>${esc(tilde(e.path))}</b><br>now ${h(e.bytes)}` + (diff ? `<br>before ${e.previous == null ? 'not present' : h(e.previous)}<br>change ${sg(e.delta)}` : '');
    return `<div class="path" title="${esc(tilde(e.path))}"><span>${esc(tilde(e.path))}</span></div><div class="track" data-tip="${esc(tipHtml)}">${diff ? '<div class="mid"></div>' : ''}${bar}</div><div class="val">${diff ? sg(e.delta) : h(e.bytes)}${note}</div>`;
  }).join('');
  el.innerHTML = `<h2>What grew</h2><p class="ink2">${head}</p>
    ${diff ? '<div class="legend" style="margin-bottom:10px"><span><i class="sw" style="background:var(--grow)"></i>grew</span><span><i class="sw" style="background:var(--shrink)"></i>shrank</span></div>' : ''}
    <div class="bars">${rows}</div>`;
})();

/* ---- Weekly job ---- */
(() => {
  const s = D.schedule, el = $('job');
  if (!s.installed) { el.innerHTML = '<h2>Weekly job</h2><p class="empty">Not installed. <code>mac-headroom schedule install --only …</code> sets up a launchd job that runs diagnose, a growth scan and the chosen cleaners every week.</p>'; return; }
  const pill = s.loaded ? '<span class="pill on">loaded</span>' : '<span class="pill warn">not loaded by launchd</span>';
  const bin = s.binary_exists ? esc(s.binary) : `${esc(s.binary)} <span class="pill warn">missing</span>`;
  el.innerHTML = `<h2>Weekly job ${pill}</h2>
    <table><tr><th>Runs</th><td>${esc(s.schedule || '?')} (or on next wake)</td></tr>
    <tr><th>Cleans</th><td>${s.cleaners.length ? s.cleaners.map(esc).join(', ') : 'all enabled cleaners'}</td></tr>
    <tr><th>Binary</th><td class="mono">${bin}</td></tr>
    <tr><th>Last run</th><td>${s.last_run ? esc(s.last_run) : 'never'}</td></tr>
    <tr><th>Log</th><td class="mono">${esc(tilde(s.log))}</td></tr></table>
    ${D.last_run_log ? `<details><summary>Last run's output</summary><pre>${esc(D.last_run_log)}</pre></details>` : ''}`;
})();

/* ---- Cleaners ---- */
(() => {
  const el = $('cleaners'), c = D.config;
  const cfg = c.exists
    ? `<span class="mono">${esc(tilde(c.path))}</span> ${c.problems.length ? '<span class="pill warn">problems</span>' : '<span class="pill on">OK</span>'}`
    : 'No config file. Built-ins only. <code>mac-headroom config init</code> writes an example.';
  const problems = c.problems.length ? `<ul>${c.problems.map(p => `<li>${esc(p)}</li>`).join('')}</ul>` : '';
  const rows = D.cleaners.map(k => `<tr>
      <td class="mono name">${esc(k.name)}</td>
      <td>${k.disabled ? '<span class="pill">disabled</span>' : '<span class="pill on">enabled</span>'}</td>
      <td>${k.source === 'config' ? 'config' : 'built-in'}</td>
      <td>${k.skip_if_running ? esc(k.skip_if_running) : '—'}</td>
      <td>${esc(k.summary)}<div class="muted">${esc(k.why_safe)}</div>${k.paths && k.paths.length ? `<div class="mono muted">${k.paths.map(esc).join('<br>')}</div>` : k.command ? `<div class="mono muted">${esc(k.command.join(' '))}</div>` : ''}${k.keep_newest != null ? `<div class="muted">keeps the newest ${k.keep_newest} per directory</div>` : ''}${k.older_than_days != null ? `<div class="muted">only when untouched for ${k.older_than_days} days</div>` : ''}</td>
    </tr>`).join('');
  el.innerHTML = `<h2>Cleaners</h2><p class="ink2">Config: ${cfg}</p>${problems}
    <div class="overflow"><table><tr><th>Name</th><th>State</th><th>Source</th><th>Skip if running</th><th>What and why</th></tr>${rows}</table></div>`;
})();

/* ---- Audit ---- */
(() => {
  const el = $('audit'), A = D.audit;
  if (!A.length) { el.innerHTML = '<h2>Deletions</h2><p class="empty">Nothing has been deleted by mac-headroom on this machine.</p>'; return; }
  el.innerHTML = `<h2>Deletions <span class="muted" style="font-weight:400">last ${A.length}</span></h2>
    <div class="overflow"><table><tr><th>When</th><th>Cleaner</th><th>Result</th><th>Size</th><th>Path or command</th></tr>
    ${A.map(a => `<tr><td>${when(a.at)}</td><td class="mono">${esc(a.cleaner)}</td><td>${esc(a.result)}</td><td class="num">${a.bytes ? h(a.bytes) : ''}</td><td class="mono">${esc(tilde(a.path))}</td></tr>`).join('')}</table></div>`;
})();

/* Tooltips for any element with data-tip */
document.addEventListener('mousemove', e => { const t = e.target.closest('[data-tip]'); if (t) showTip(t.dataset.tip, e.clientX, e.clientY); else if (!e.target.closest('#hsvg')) hideTip(); });
</script>
</body>
</html>
"##;
