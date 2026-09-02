---
title: MilkDrop
description: Run the MilkDrop visualiser, install presets, and use its controls.
nav_order: 5
---

Open MilkDrop from the top-bar visualiser button, Ctrl+Shift+K, Settings, or
the mini player's **V** menu. It uses
[projectM](https://github.com/projectM-visualizer/projectm) to play `.milk`
presets in its own window and process.

<video autoplay loop muted playsinline preload="metadata" poster="/assets/images/milkdrop-poster.jpg" aria-label="MilkDrop running in Fastpotify" style="width: 100%; height: auto;">
  <source src="/assets/images/milkdrop.mp4" type="video/mp4">
</video>

MilkDrop is included in the Linux, macOS, and x86_64 Windows builds. The
Windows on ARM build leaves it out.

## The window

Drag the image to move the window. Double-click or press **F** for fullscreen.
Press **Esc** to leave fullscreen or close the window. Drag the lower-right
corner to resize it.

MilkDrop uses the same post-equalizer, pre-volume audio as the other
visualisers. It keeps moving at zero volume and stays flat when another device
is playing.

## Presets

Presets change every ten seconds by default. Change the interval in Settings.
Presets live in the config directory's `milkdrop` folder. The first time
MilkDrop opens with an empty folder, Fastpotify automatically downloads the
550 MilkDrop 2 presets and the 9,800-preset Cream of the Crop pack. Settings
can fetch either pack again. Until the first preset arrives, projectM shows
its idle preset.

## Controls

- **N** or right arrow: next preset.
- **P** or left arrow: previous preset.
- **H**: switch on the next beat.
- **L**: keep the current preset.
- **R**: switch between random and folder order.
- **T**: show or hide the preset name.
- **D**: show or hide the frame rate.
- **I**: cycle the song display between a change notification, always visible,
  and hidden.
- **?** or **F1**: show all shortcuts.

The normal playback shortcuts also work.
