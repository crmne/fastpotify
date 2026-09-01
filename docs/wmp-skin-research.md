---
title: WMP Skin Research
description: Findings from researching original Windows Media Player skin (.wmz/.wms) support in Fastpotify.
---

## WMP Skin Runtime Research

This document records the Milestone 0 research for loading original Windows
Media Player skin packages (`.wmz` / `.wms`) in Fastpotify: what the format
is, what real skins contain, what exists elsewhere, and what of Fastpotify's
Winamp infrastructure can be reused. Every corpus number below was measured
on real `.wmz` files, not read off documentation.

### 1. The format, verified

A `.wmz` is a plain zip archive renamed — the same container as a `.wsz`.
Microsoft's own guide: compress the skin files into a `.zip`, rename it to
`.wmz`. A `.wms` file inside is the skin definition; the rest is art
(bitmaps, GIFs), one or more `.js` JScript files, and occasionally sounds.

The `.wms` file is XML-flavoured but *not* reliably well-formed XML:

- Root element is `THEME`, containing one or more `VIEW` elements. Each
  `VIEW` is one window; `SUBVIEW` layers sub-regions within a view.
- Attribute values may be literals, `wmpprop:` bindings
  (`value="wmpprop:player.settings.volume"`), or `jscript:` inline
  expressions (`left="jscript:treble.left+treble.width/2-15;"`).
- Files may be UTF-8/Latin-1 *or UTF-16*. One of the sampled skins
  (`Revert/netgen.wms`) is UTF-16 with a BOM. A parser must detect the BOM.
- Real skins contain malformed XML: duplicate attributes, mis-nested
  elements, undeclared encodings, `res://wmploc.dll/RT_TEXT/#132;` scheme
  references in `scriptFile`. 16 of the 22 skins sampled parse with a
  strict XML parser; the rest need a forgiving reader.
- Element and attribute names are case-insensitive in practice (skins mix
  `BUTTONGROUP`, `buttongroup`; `transparencyColor`, `transparencycolor`).

Element model (from Microsoft's archived Skin Programming Reference, which
is still online):

```
THEME                          author, copyright, title, currentViewID, version
 └── VIEW                      width, height, backgroundImage, transparencyColor,
     │                         backgroundColor, titleBar, resizable, scriptFile,
     │                         timerInterval, onLoad/ontimer/onclose …
     ├── SUBVIEW               same background attributes + left/top/width/height
     ├── IMAGE                 positioned bitmap
     ├── BUTTON                image, hoverImage, downImage, disabledImage,
     │                         hoverDownImage, transparencyColor, sticky (toggle)
     ├── BUTTONGROUP           image/hoverImage/downImage/disabledImage for the
     │   └── BUTTONELEMENT       whole group + mappingImage; each child names a
     │                           mappingColor key into that bitmap
     │                         Predefined children: playelement, pauseelement,
     │                         stopelement, prevelement, nextelement, ffwdelement…
     ├── SLIDER                backgroundImage, thumbImage(+hover/down/disabled),
     │                         direction, tiled, borderSize, min, max, value,
     │                         onDragBegin/onDragEnd
     │                         Predefined: seekslider, volumeslider, balanceslider
     ├── TEXT                  value, fontFace, fontSize, fontStyle, foregroundColor,
     │                         justification, scrolling*; predefined: statustext,
     │                         currentpositiontext, durationtext
     ├── EFFECTS               WMP visualization host (currentEffectType, presets)
     ├── PLAYLIST, VIDEO, EDITBOX, LISTBOX, POPUP, AUTOMENU,
     ├── CUSTOMSLIDER, PROGRESSBAR, EQUALIZERSETTINGS, VIDEOSETTINGS
     └── PLAYER / CONTROLS / SETTINGS   script-only access objects
```

Position and size come from the *ambient* attributes every element shares:
`left, top, width, height, zIndex, visible, enabled, id,
transparencyColor`-independent `clippingImage/clippingColor` (clip a control
to a shape), `alphaBlend`, horizontal/vertical alignment. Coordinates are
absolute pixels within the view; views are fixed-size unless `resizable`.

### 2. What a real corpus contains

22 skins were pulled from the w2krepo mirror of the historical Microsoft
skin gallery (Toothy, Classic, Goo, Headspace, Melvin, Optik, Revert,
Roundlet, Rusty, Stealth, WinME, Zengarden, circle, claw, gadget, polygon,
9SeriesDefault, Asimov Radio, anemone, Cablemusic, Ducky, Heart). These
span WMP 7–9 eras, the shapes Microsoft shipped. A larger collection
(160+ skins) exists on archive.org (`windowsmediaplayerskinscollection`)
and a mirror list at w2krepo.somnolescent.net.

**Package contents:** `.wms` + `.js` (1–7 KB each, all of them) + art. Art
is overwhelmingly 24-bit BMP; GIF appears in webby skins; PNG is rare in
this era (1 file across 22 skins). WMP11-era skins (not in this sample)
add PNG art but still use colour keys for shape.

**Element frequency (opening tags across all 22 skins):**

| Element | Count | Element | Count |
|---|---|---|---|
| BUTTONELEMENT | 449 | VIEW | 28 |
| TEXT | 231 | THEME | 22 |
| SUBVIEW | 218 | EFFECTS | 21 |
| SLIDER | 161 | PLAYER | 21 |
| BUTTON | 110 | VIDEO | 20 |
| BUTTONGROUP | 74 | EQUALIZERSETTINGS | 18 |
| predefined buttons/elements | ~120 | PLAYLIST | 16 |
| CUSTOMSLIDER | 5 | EDITBOX/LISTBOX/POPUP | 0–2 |

The transport controls are overwhelmingly predefined elements
(`playelement`, `stopelement`, `nextelement`, `prevelement`,
`pausebutton`…) and predefined controls (`seekslider`, `volumeslider`,
`statustext`, `currentpositiontext`, `durationtext`). These carry their
behaviour built in — **no script required**.

**Attribute frequency:** `id, top, left, onclick, mappingcolor, sticky,
transparencycolor, width, value, backgroundimage, height, tooltip, image,
uptooltip, hoverimage, visible, onchange, zindex, foregroundcolor,
downimage, fontsize, bordersize, thumbimage, min, max` lead the list.
`mappingcolor` (517) out-ranks almost everything: the colour-mapped
button group is *the* WMP control idiom.

**Script usage — the key numbers:**

- 22 of 22 skins reference a `scriptFile` (all ship a `.js`, 1–7 KB);
  one (`Revert`) never *uses* script — no handlers, no bindings.
- `wmpprop:` bindings appear in every scripted skin: `value` (188),
  `enabled` (24), `max` (16), `down` (6), `visible`, `left/top/width`.
  The bound properties are a small closed set: `player.settings.volume`,
  `player.settings.balance`, `player.settings.mute`,
  `player.controls.currentposition(,string)`,
  `player.currentmedia.(name|duration|durationstring|sourceurl)`,
  `player.network.downloadprogress`, `eq.gainlevel1..10`,
  `eq.currentpresettitle`, `mediacenter.effect(type|preset)`,
  `viseffects.currentpresettitle`.
- `jscript:` appears mostly in *layout* attributes: `left` (150), `top`
  (125), `height` (27), `width` (45), plus colour/fontsize/tooltip values.
  The expressions are almost all `id.attr + id.attr ± constant` —
  positional arithmetic over sibling elements, e.g. the bass slider placed
  relative to the treble slider.
- `onClick` handlers: the most common values across the corpus are
  `view.minimize()` (21), `view.close()` (21),
  `view.returnToMediaCenter()` (21), `eq.reset()` (10),
  `visEffects.previous()/next()` (10), playlist/EQ toggles, plus named
  script functions (`TogglePl()`, `SetVisibility(noPane)`…) and the rare
  direct call (`player.controls.next()`). Predefined elements need no
  handler at all.

**Window shape:** no skin in the sample uses a separate mask bitmap. The
visible *and clickable* region is the background image of the view (or a
full-coverage subview) with its `transparencyColor` keyed out — magenta
`#FF00FF` in nearly every skin. Transparent pixels are outside the window:
clicks pass through. Toothy's tooth shape, circle's circle, claw's claws
are all colour keys on a background layer. (The ambient
`clippingImage/clippingColor` pair exists for clipping *controls*, and
alpha PNG art is blended per-pixel in later versions, but colour-keyed
backgrounds define the window.) A view may also omit `width`/`height`
entirely — claw and Heart do — in which case the window is the size of
its background image.

**Sliders:** track is `backgroundImage` (tiled along `direction`,
inset by `borderSize`); position is `thumbImage` drawn at the value
offset; `value` is bound to player state via `wmpprop:`; `onDragEnd`
writes it back (`player.controls.currentposition=value;`). Seek sliders
bind `max` to `wmpprop:player.currentmedia.duration`. This maps 1:1 onto
Fastpotify's existing slider handling.

**Button states:** up (`image`), hover (`hoverImage`), down
(`downImage`), hover-while-down (`hoverDownImage`), disabled
(`disabledImage`), and toggle via `sticky`. Button groups render from one
bitmap per state (`image`/`hoverImage`/…on the group) and hit-test
through the `mappingImage` bitmap: the pixel colour under the pointer
selects the `buttonelement` whose `mappingColor` matches.

**Text:** font face/size/style/colours, `justification`, scrolling
(amount/delay/direction — a marquee, like Winamp's). Values bind via
`wmpprop:` (`player.currentmedia.name`) or are filled by script
(Toothy's time display is written by `OnTimerTick()`).

### 3. Existing implementations

**The working hypothesis is confirmed: no mature, generic, open-source
`.wmz/.wms` runtime exists.** Everything found is one of:

- *Winamp* runtimes (Webamp, Fastpotify, spot-the-winamp, cranamp…) —
  the `.wsz` problem is solved repeatedly; none read `.wms`.
- *Hand-built WMP lookalikes* — rmellis' WMP 7/8/9 web clones, WMPotify,
  98.js etc. recreate WMP's chrome with CSS/JS but load no original skins.
- *Per-skin ports* — PlayerX (Toothy port), forum recreations.

**SkinDoc2** (SourceForge, pre-alpha, 2006–2007) ships **binaries only**:
no parser or renderer source is published. Its data files are still
useful: each `.sel` "widget" (zipped `widget.xml`) is a complete
attribute table for one WMP element — name, value type, default,
read/write, player version — and the `.stf` templates cover WMP 7/8/9/10
and Winamp. Treat it as a machine-readable cross-check of the SDK
element model, not reusable code. `WMZOpen` is just a zip browser.

### 4. Fastpotify infrastructure that carries over

- **`src/skin/zip.rs`** — dependency-free zip reader with size/encryption
  guards and a writer. A `.wmz` is the same archive; reusable as-is.
- **`src/skin/mod.rs` `Bitmap`** — BMP/PNG decode via `image`, already
  tolerant of mislabelled files; GIF decode comes free with the `image`
  crate. Case-insensitive, folder-tolerant file lookup in the archive
  matches how WMP resolves skin files too.
- **`src/skin/config.rs` `Mask`** — row-span mask (`spans(y)`,
  `contains(x, y)`) built from polygons. A WMP window mask is the same
  structure with a different source: key out `transparencyColor` (or
  alpha < threshold) rows of the background bitmap into spans.
- **`src/winamp.rs`** — the async load/install/poll pattern, texture
  caching per window, scale 1–4×, position restore, skin folder listing.
  A `WmpState` wants the same shape (loading a 58 MB Alienware skin must
  not touch the UI thread).
- **`src/ui/winamp/mod.rs` `View`** — masked sprite painting (per-row
  spans against a texture region) and rect interaction via
  `ui.interact` with `Sense::click`/`click_and_drag`, hover cursor,
  tooltips. WMP rendering is bitmap-at-position rather than sprite
  sheets, so the paint helper simplifies; interaction and masking carry
  over directly. New capability needed: colour-map hit testing
  (pointer position → pixel of `mappingImage` → element), which is a
  simpler cousin of the existing mask work.
- **Actions** — `src/model.rs` `Action` is already the media-abstraction
  layer; a WMP `SkinAction` should just produce `Action`s.

What does *not* carry over: `sprites.rs`/`layout.rs` (fixed Winamp
geometry), `font.rs`/`pixel_text` (Winamp's 5×6 bitmap font; WMP text is
system fonts), playlist/equalizer windows (Winamp-specific layout).

### 5. What an MVP can honestly cover

Corpus-driven, the evidence supports this subset:

1. **Parse:** BOM/encoding detection, forgiving XML reader, element tree
   `THEME → VIEW → SUBVIEW → {IMAGE, BUTTON, BUTTONGROUP/BUTTONELEMENT,
   SLIDER, TEXT}`, ambient attributes, `wmpprop:`/`jscript:` recognised
   as value kinds. Unknown elements/attributes: recorded, skipped.
2. **Render:** background layers with colour-key transparency, buttons
   (4-state), button groups (per-state bitmaps + mapping image),
   sliders (tiled track + thumb), text with justification and basic
   scrolling. `EFFECTS` boxes can render Fastpotify's own visualiser
   later; `PLAYLIST`/`VIDEO` render as placeholders or hide.
3. **Bind without script:** predefined elements/buttons/sliders/text
   carry built-in actions; `wmpprop:` bindings cover value/enabled/max
   state in and out; the ~8 common literal `onClick` forms
   (`view.minimize/close/returnToMediaCenter`,
   `player.controls.play/pause/stop/next/previous`, `theme.openView(id)`)
   are pattern-matched to `Action`s.
4. **Layout arithmetic:** a tiny evaluator for `jscript:`
   `id.attr ± id.attr ± constant` (and bare `wmpprop:` in numeric
   attributes) resolves the remaining layout attributes; anything
   unevaluatable falls back to 0 and logs.

From the corpus this yields a *correct-looking, partially interactive*
render of roughly: 5/22 skins with zero script dependence beyond the
above (Revert, Melvin, AdvancedDefault, Zengarden, polygon — no
`jscript:` at all), and the rest correct except elements whose visibility
or content a `.js` toggles (drawers, balloons, panes default to their
`visible` attribute — Toothy's drawers start closed, which is right).

**Explicitly out (security + scope):** no JScript execution, no
`res://` resolution, no `openDialog`/`playSound`/registry preferences,
no WMP plugin/visualization compatibility. Skins are untrusted input:
zip limits already in `zip.rs`, image dimension caps, recursion caps,
and no filesystem access from skin content.

### 6. Suggested milestones (revised by evidence)

1. **Parser + inspector**: `Skin::wmp(path)` producing a `SkinDocument`
   IR + a `--inspect-wmp-skin` style dump; tests over the corpus.
2. **Static render**: VIEW/SUBVIEW/IMAGE/TEXT with colour-key masking;
   Toothy shows its tooth.
3. **Controls**: BUTTONGROUP mapping hit-testing, BUTTON states, SLIDER
   drag → `Action`; predefined elements wired.
4. **Bindings**: `wmpprop:` two-way for volume/seek/mute/EQ; layout
   arithmetic evaluator.
5. **Fidelity**: hover/down states, tooltips, text scrolling, scale,
   secondary views (`theme.openView`) as separate windows if wanted.
6. **Script phase (optional, later)**: restricted `player`/`view`/
   `theme` shim behind a sandboxed JS engine, only if the corpus demands.

### 7. Corpus sources

- Mirror of the Microsoft gallery: `https://w2krepo.somnolescent.net/Windows%20Media%20Player/Skins/`
- Archive.org collection (160+ skins, thumbnails):
  `https://archive.org/download/windowsmediaplayerskinscollection`
- WMP 7 skin gallery snapshot (Windows Me Bonus Extras):
  `https://archive.org/details/wmeupd-extras`
- Skin archive site: `https://wmpskinsarchive.neocities.org/`

### 8. Reference

- Archived SDK (elements/attributes): `https://learn.microsoft.com/en-us/previous-versions/windows/desktop/wmp/windows-media-player-skins`
  (start at *Skin Programming Reference*; `THEME`, `VIEW`, ambient
  attributes, `BUTTON`, `BUTTONGROUP`, `BUTTONELEMENT`, `SLIDER`, `TEXT`)
- Packaging: "Complete Code for Simple Skin" (same SDK section) — zip →
  rename `.wmz`.
- Note: Microsoft has announced WMP skins stop working in WMP Legacy on
  Windows 11 24H2+ from November 2026 — the compatibility niche this
  targets is about to have no native host at all.
