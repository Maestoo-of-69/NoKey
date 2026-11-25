# NoKey 0.3 design: compact list

One system for every window. Direction: a compact, keyboard-first list in the
spirit of launcher apps (dense rows, keycaps, one bottom action bar), drawn
with plain GDI. Principles taken from the taste-skill and emil-design-eng
skills, translated to Win32:

- **One system.** One spacing scale, one radius family, one button pair
  (primary + ghost), one row recipe, one hairline, one caption style.
- **Restraint.** No gradients, shadows or decoration (the Jewish theme keeps
  its header lattice and star as its identity). At most one primary button
  per window.
- **Instant feedback, quiet motion.** Every pressable thing reacts at once
  (press-in is instant). Small ease-out transitions (100–200 ms) only soften
  what the mouse changes; keyboard actions, global hotkeys and opening a
  window never animate (see "Motion").
- **Readable everywhere.** Text contrast stays at the levels the theme tests
  enforce; long Russian / "pain" strings and Hebrew RTL must fit (measure,
  never hard-code a label width); everything goes through `px()`.

## Tokens

Spacing (96-DPI units): 4, 8, 12, 16, 24, 32. Window side padding 16.
Radius: 6 for controls, keycaps and selected rows; 8 for fields and banners;
pills (switch) use the full height.

Type (Segoe UI, `ui::fonts()`):

| Font | Size / weight | Use |
| --- | --- | --- |
| `title` | 18 semibold | window title in the header bar |
| `bold` | 14 semibold | row names, button labels, field values that matter |
| `normal` | 14 regular | body text, inputs |
| `small` | 12 regular | summaries, hints, status, captions |
| `caps` | 11 semibold | keycap labels |
| `big` | 16 semibold | rare: empty-state headline |

Captions are sentence case (`small`, `muted`), never upper case.

Palette fields (`ui::theme::Palette`): `bg`, `surface` (fields, cards),
`selected` (selected row), `hover` (row and ghost-button hover), `text`,
`muted`, `border` (control outlines), `line` (hairlines), `accent` (primary
button fill, focus ring, switch on), `accent_text`, `warn` / `warn_text`
(the "all binds off" banner), `key` / `key_edge` (keycap fill and its 2px
bottom edge), `ok` (connected dot, success tint), `err` (failure tint; white
in Contrast), plus the Jewish-only `pattern` / `star`. Motion tints mix
`theme::TINT` (0.35) of `accent` / `ok` / `err` into the row or button color.

## Components (in `ui/mod.rs` and `ui/kit.rs`)

- **Primary button** (`BTN_PRIMARY`): accent fill, `accent_text`, radius 6,
  height 32. Hover: the fill fades 6 % toward `accent_text` (a lighter
  accent). Pressed: 1 px lower, fill 15 % toward `key_edge`.
- **Ghost button** (`BTN_SECONDARY`): no fill, no outline at rest; `hover`
  fill fades in on hover; pressed: 1 px lower, 15 % toward `key_edge`;
  focus = 1px accent outline.
- **Dropdown** (`BTN_DROPDOWN`): ghost button with label + `▾` in muted.
- **Icon button** (`BTN_ICON`): 32x32 ghost button with one glyph (⚙, ⋯).
- **Switch** (`BTN_SWITCH`): pill 36x20, knob 14; on = accent track,
  `accent_text` knob; off = `border` outline, `muted` knob. Label (if any)
  to the left. Checked state = on.
- **Tab** (`BTN_TAB`): label in `muted` (fades to `text` on hover);
  selected = `text`. Tab buttons are 34 tall; the 2px accent underline is
  drawn by the window in the strip below them (`kit::tab_underline`) so it
  can slide between tabs.
- **Slider** (`kit::slider`, own window class): 4px rail in `border`, the
  part up to the value in `accent`, round 14px thumb (`accent_text` fill,
  `accent` outline), focus = 1px accent ring around the thumb. Log scale
  (30..30000 snaps to 5 below 300, 10 below 3000, 100 above); ←/→/↑/↓ ±1 %, PageUp/Down
  ±10 %, Home/End, mouse wheel ±2 %. The value label ("300 chars/min",
  `small`) is drawn by the parent to the right. Mirrored in Hebrew.
- **Checkbox** (`BTN_TOGGLE`): 16x16 box radius 4; the accent fill and ✓
  fade in together.
- **Field frame** (`edit_frames`): `surface` fill, 1px `border`, radius 8;
  focus = 1px accent. Padding 10 x 8 around the edit control.
- **Keycaps** (`kit::keycaps`): a shortcut label is split into keys on
  " + " and " → " (and "  |  " between alternatives). Each key is a box:
  `key` fill, 1px `border`, 2px `key_edge` bottom, radius 6, height 22,
  min width 22, 7px side padding, `caps` font. "+" joins are drawn as 4px
  gaps, "→" as a muted small arrow between keys. Right-aligned in rows.
  `kit::keycaps_ex` adds a hover amount (border toward `text`), a pressed
  look (no bottom edge, caps 1 px lower) and a fade-in of the last key.
- **Row** (binding list, profile list): height 40, side inset 4 inside the
  list. Rest: no fill (list background = `bg`). Selected: `selected` fill
  rounded 6. Icon square 24 (radius 6, `surface` fill, `bold` letter or
  emoji). Name in `bold`, summary in `small muted` on the same line after
  the name when it fits, otherwise ellipsis. Disabled/paused rows draw the
  name in `muted` and show an "Off"/"Paused" text chip instead of keycaps.
  No per-row separators.
- **Header bar**: 52 tall, `bg`, 1px `line` at the bottom. Title at x 16.
  The Jewish theme draws `header_band` + star inside it (taller, 96).
- **Banner** (`kit::banner`): full-width strip under the header, `warn`
  fill, `warn_text`, centered `small` text, height 32. Only shown when all
  binds are off or the emergency switch failed.
- **Action bar**: bottom strip, 1px `line` on top, height 56; buttons 32 tall
  centered vertically, 16 from the sides, 8 apart.
- **Status dot**: 8px circle, `ok` when connected, `border` outline when not.

## Windows

There is one window. It opens at 520 x 620 (the minimum) in every theme (a
taller themed header takes its space from the binding list) and can be
dragged larger or maximized: views read `page::width()` / `height()`, controls
placed once follow with `page::anchor` (right-aligned ones move, fields
stretch, the action bar stays at the bottom), and a rebuild for a new language
keeps the size. The editor and Settings are child **pages**
covering it (`ui/page.rs`), one at a time; the main view's controls are hidden
meanwhile. A page header reads `NoKey / <title>`: "NoKey" is a button
(`BTN_LOGO`) back to the main view. It asks before dropping unsaved binding
edits, as does opening another page over them; Cancel and Esc do not ask.

### Main

1. Header bar: "NoKey" title; right side, right to left: master switch
   (`BTN_SWITCH`, checked = binds enabled, tooltip `tip.disable_all`),
   profile dropdown (ghost), Discord status dot + short text (small, muted).
2. Banner when binds are off (`main.status_disabled`) or on emergency error.
3. Search row (y = header + banner + 12): search field fills the width
   minus a 120 sort dropdown on the right (8 gap).
4. Caption row: `main.bindings` left, `main.active_count` right.
5. List: fills down to the action bar; rows as above. Empty state: centered
   `big` headline + `small muted` hint, no box.
6. Detail line just above the action bar (24 tall, small muted): the
   selected binding's actions summary · trigger · profile, or the last run
   result ("✓ …" / "✕ …") while a flash or result is fresh.
7. Action bar: `+ New binding` (primary), Edit (ghost), More `⋯`
   (ghost dropdown) on the left; Run `▶` (ghost) and Settings `⚙`
   (`BTN_ICON`) on the right. Run keeps its cooldown / flash behaviour.

The "DISABLE ALL BINDS" big button and the SELECTED / STATUS sections go
away; their information moves into the switch, banner and detail line. All
existing commands, menus, tooltips and keyboard behaviour stay.

### Editor and Settings

Page header bar (52 tall, see above), same field frames, ghost/primary
buttons and action bar (Cancel ghost + Save primary on the right; Settings
keeps its own buttons in the bar). Tabs sit right under the header bar,
16 from the sides. Sections use captions (sentence case) with 24 above and
8 below. Shortcut previews use keycaps. Window sizes may change; content
must not be cut at 100 % and 150 % DPI in en, ru, pain and he.

## Motion

Engine: `ui/anim.rs`. Tweens are keyed by (window, slot) and exist only while
running; paint asks `anim::value(hwnd, slot, rest)` and gets the logical state
when nothing runs. Curves are cubic ease-out (`1 - (1 - t)^3`), except
knobs and underlines, which overshoot by about 10 % (`anim::spring`); a
transition interrupted mid-way restarts from its current value. Tweens start
only in mouse handlers (`kit/track.rs` subclasses, or a command handler that
checks `anim::by_pointer()`), so keyboard actions, refreshes, global hotkeys
and freshly opened windows snap. One 15 ms timer per window runs while that
window has a tween or a live pulse and kills itself on the first idle tick
(or as soon as the window is hidden or minimized). Pulses repaint only their
own rect and expire 250 ms after paint stops asking for them. Owner-drawn
buttons and rows draw into a memory DC and are copied in one BitBlt
(`ui::buffered_item`), so per-frame repaints do not flicker.

| # | Effect | Duration | Trigger |
| --- | --- | --- | --- |
| 1 | Button hover: fill fades (ghost bg → `hover`; primary → lighter accent) and a 2px ring appears (ghost: `accent`; primary: its text colour) | 180 ms | mouse enter/leave |
| 2 | List row hover glide; a 3px `accent` bar grows from the middle (stays on the selected row) | 220 ms | mouse move |
| 3 | Tab label `muted` → `text` on a `hover` pill | 180 ms | mouse enter/leave |
| 4 | Keycaps of a hovered row: border 35 % toward `text` | with #2 | mouse move |
| 5 | Button press: each side moves in 2, 25 % toward `key_edge`; release fades back | instant / 240 ms | mouse down/up |
| 6 | Keycaps of a held row look pressed | instant | mouse down until up/leave |
| 7 | Selection flash: `selected` + 60 % `accent` → `selected` | 600 ms | mouse click that changes the selection |
| 8 | Switch knob springs across and stretches on the way, track `border` → `accent` | 280 ms | mouse click (incl. its label in Settings) |
| 9 | Tab underline springs between tabs, passing the target by at most 6 (editor, settings) | 320 ms | mouse click on a tab |
| 10 | Checkbox shrinks and pops while fill and ✓ fade | 280 ms | mouse click |
| 11 | Binding ran: its row flashes `ok` / `err` tint and fades | 1100 ms | any run (hotkey too), only while the main window is on screen |
| 12 | Run button crossfades to the result tint with "✓ Done" / "✕ Failed", back after the 1.5 s result | 200 ms | Run clicked with the mouse (hotkey runs: tint snaps) |
| 13 | Banner height 0 ↔ 32, the body moves with it (DeferWindowPos) | 200 ms | mouse click on the master switch |
| 14 | Discord status dot pulses toward `bg` | 1 s period | while the token is being checked |
| 15 | Record button pulses (ghost: toward `warn`; warn: dims toward `bg` and back) | 1.2 s period | while recording (editor, profile switch) |
| 16 | Newest recorded key fades in | 240 ms | each key the recorder adds |
| 17 | Slider: thumb grows by 2 with a halo (`bg` + 32 % `accent`); press tints the thumb 15 % toward `key_edge`, release fades back | 180 ms / instant / 240 ms | mouse enter/leave, mouse down/up (drag and keys snap) |
| 18 | Click ripple: a circle spreads from the clicked point and fades (push buttons) | 480 ms | mouse down |
| 19 | Page slides in from the side (64) | 280 ms | a page opened with the mouse |
| 20 | "NoKey" button: name turns `accent`, a line grows under it; hand cursor | 180 ms | mouse enter/leave, keyboard focus |

**Opt-out:** with Windows "Animation effects" off (Settings → Accessibility →
Visual effects; `SPI_GETCLIENTAREAANIMATION`, re-read every 2 s) every
transition jumps to its end state, pulses stay solid and no timer starts.

## Themes

All five stay (Dark, Light, Contrast, System, Jewish), each mapped onto the
token list above. Contrast keeps black / white / yellow only.
