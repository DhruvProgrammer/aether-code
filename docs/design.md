---
title: "AETHER — Design System"
status: "canonical"
audience: "UI / TUI / docs contributors"
summary: "Minimalist, light, low-chroma UI language built on official Pantone references. Terminal-safe, SEO-structured markdown, no heavy colours."
---

# Design System — `aether` / `aether-mind`

> **Principle:** *Quiet by default.* One ink, one accent, lots of air. Colour is used to
> signal state, never to decorate. Every surface is light. Contrast is earned by
> typography and whitespace, not by saturation.

This document is the single source of truth for the look of the agent binary, its TUI,
its docs site, and its `README`/`AGENTS.md`/`CONTEXT.md` family. It mirrors the minimalism
of OpenCode's desktop shell, the RAM-light restraint of jcode, and the fullscreen calm of
grok-build — but with an explicitly **Pantone-anchored, very-light** palette.

---

## 1. Palette (Pantone-anchored, light)

| Token | Role | Pantone (TCX) | HEX | ANSI (TUI) | Notes |
|---|---|---|---|---|---|
| `--canvas` | App background | 11‑0601 *Bright White* | `#F6F7F9` | `255` (#f6f6f6) | Never pure black UI |
| `--surface` | Cards / panels | 11‑0701 *Whisper White* | `#FFFFFF` | `231` | Raised 1px hairline |
| `--surface-sunk` | Inputs / code | 11‑0601 *Bright White* | `#FAFBFC` | `255` | Slight inset feel |
| `--ink` | Primary text | 19‑3911 *Pavement* | `#1F2024` | `240` | Near-black, not black |
| `--ink-soft` | Secondary text | 14‑4102 *Coastal Mist* | `#6B7280` | `245` | Captions, metadata |
| `--ink-faint` | Tertiary / hints | 14‑4306 *Cloud Blue* | `#9AA1AC` | `248` | Placeholders |
| `--hairline` | Borders / rules | 14‑4102 *Coastal Mist* | `#E6E9ED` | `254` | 1px, never bold |
| `--accent` | Primary action | 14‑4318 *Still Blue* | `#A7C7D6` | `117` | **Single** accent only |
| `--accent-ink` | Text on accent | 19‑3911 *Pavement* | `#22323B` | `240` | Readable on light blue |
| `--ok` | Success | 13‑0117 *Mint* | `#BFE3D0` | `120` | Validation passed |
| `--warn` | Caution | 12‑0825 *Butter* | `#F2E2A8` | `222` | Ask / pending |
| `--danger` | Destructive | 16‑1726 *Coral* | `#E7B7B0` | `174` | Deny / error |
| `--pin` | Pinned memory | 15‑3905 *Cascade* | `#C9C3DE` | `147` | "never delete" marker |

**Chroma budget:** no token may exceed ~18% saturation at full strength. Accent is used for
*one* focal element per screen (the active prompt, the primary button). Everything else is
greyscale-on-white. This is what keeps it "very light, no heavy colours."

### Tints (for hover / selected states, all ≤8% opacity overlays)
- `--accent-wash`: `Still Blue` @ 8% over canvas → `#EEF4F7`
- `--ink-wash`: `Pavement` @ 6% over surface → `#F0F1F3`

---

## 2. Typography

| Layer | Family | Size | Weight | Tracking |
|---|---|---|---|---|
| Display (TUI title) | System mono / `Berkeley` | 18px | 600 | -0.2 |
| Body / chat | System UI or mono | 14px | 400 | 0 |
| Code / tool I/O | `Mono` (Cascadia/JetBrains) | 13px | 400 | 0 |
| Caption / meta | System UI | 12px | 400 | +0.3 |

**Rules**
- One type family per surface. Chat = proportional; tool output = monospace. Never mix in one block.
- Line-height 1.5 for prose, 1.35 for code.
- No all-caps body text. UPPERCASE only for 2–4 char state pills (`OK`, `ASK`, `DENY`).

---

## 3. Spacing & Layout (8pt grid)

`4 · 8 · 16 · 24 · 40 · 64`. The TUI keeps a 16px gutter on each side; docs keep a
max content width of **72ch** for prose and **96ch** for code/diagrams. Generous vertical
rhythm is the main "luxury" signal — do not fill the whitespace.

---

## 4. Components (TUI + Docs)

### 4.1 State pill
```
[ OK ]  [ ASK ]  [ DENY ]  [ PIN ]
```
Light fill, 1px hairline, 4px radius, uppercase 11px. No shadow.

### 4.2 Tool activity row
`--accent-wash` background, mono 13px, `--ink-soft` label, `--hairline` divider between rows.
Streaming rows show a single `·` pulse in `--accent`.

### 4.3 Diff viewer
- Added: `--ok` left rule (3px), text unchanged colour.
- Removed: `--danger` left rule (3px), struck softly.
- No full-red / full-green backgrounds. Light, rule-based only.

### 4.4 Prompt input
`--surface-sunk` field, 1px `--hairline`, focus ring = `--accent` 2px @ 40%. Caret = `--ink`.

---

## 5. Docs / SEO Structure

Every markdown file in this repo follows a strict, machine-readable shape so it is
**discoverable and diff-friendly** (the "SEO" requirement = structured, linkable, greppable):

1. **Frontmatter** (YAML) at the very top:
   ```yaml
   ---
   title: "Short noun phrase"
   status: "draft | review | canonical"
   audience: "who edits this"
   summary: "one line, used for index/og:description"
   ---
   ```
2. **One H1** = title. **H2** = sections. **H3** = subsections. No skipped levels.
3. **Tables** for reference data (palette, config keys, phases). **Fenced code** for schemas.
4. Each file ends with `## See also` linking siblings (internal anchors only).
5. No emoji in docs unless the user requests. ASCII markers only (`[OK]`, `[!]`).

This makes the doc set indexable by any static site generator and trivially greppable by the
agent itself (the agent can read its own `CONTEXT.md` / `AGENTS.md` per §12 of the spec).

---

## 6. Anti-patterns (do not ship)

- ❌ Dark mode as the *default* (offer as opt-in only; light is canonical).
- ❌ Saturated brand colours, gradients, glows, heavy shadows.
- ❌ More than one accent colour on screen at once.
- ❌ Decorative icons without state meaning.
- ❌ Full-bleed coloured backgrounds behind chat text.

---

## See also
- [plan.md](./plan.md) — phased build, Phase 1 = minimal working agent
- [context.md](./context.md) — project context & memory layers
- [skills.md](./skills.md) — skill discovery/loading using this palette for markers
- [architecture.md](./architecture.md) — crate layout (jcode/grok-build inspired)
- [roadmap.md](./roadmap.md) — `.exe` packaging on Windows
