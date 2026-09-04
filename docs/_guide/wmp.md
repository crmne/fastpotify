---
title: Windows Media Player Skins
description: Wear classic Windows Media Player (.wmz) skins as a transparent window.
nav_order: 7
---

Open the skin window with **Switch to it** in Settings, under Windows Media
Player skins. It wears classic Windows Media Player `.wmz` skins: the window
takes the skin's shape, transparent where the skin leaves nothing, and drags
by its shape. Only one player window is open at a time. The skin's close or
return button, or Ctrl+Esc (Cmd+Esc on macOS), brings the main window back.

## Skins and window size

Drop a `.wmz` file on either window to install and use it. Settings lists the
installed skins and can open the skins folder. Community archives of the
original gallery mirror many skins, including the
[Internet Archive collection](https://archive.org/details/windowsmediaplayerskinscollection).

The skin window uses whole-number scaling to keep pixels sharp, 1x to 4x in
Settings, or Command (or Control) with plus and minus inside the skin window.
Fastpotify remembers the window position.

Skins position their panels with a little arithmetic (`treble.left` and the
like) and with numbers their scripts declare; both are honoured, so drawers
and panes stand where the skin puts them. A skin whose script a reader cannot
follow still opens — its panes stand where its attributes put them.

## Main controls

Transport buttons (play, pause, stop, previous, next), volume and seek
sliders, mute, shuffle, and repeat all work, with hover and pressed states
and tooltips where the skin names them. Button groups hit-test through their
mapping image, the way the player did. Minimize, close, and return-to-center
do what they say. Sliders drag; a click on the media pane cycles the
visualiser, like Winamp's display.

The media pane — the screen a skin reserves for video or a visualiser —
shows the bars or the scope, in the classic palette, scaled to the pane. Turn
on **MilkDrop picture** in Settings to wear MilkDrop's own picture there
instead, rendered by a hidden child; off is the built-in bars or scope, which
cost almost nothing. The **vis** setting picks bars, scope, or off. Clicking
the pane cycles through them. The picture follows the equalizer and never the
volume knob, so it still dances at zero volume, and it stays flat when
another device is playing. A skin without a way back still leaves one:
Control (or Command) with Escape always returns to the main window.

Video itself does not play; the pane shows the visualiser instead. Playlist
and equalizer panes a skin defines are not drawn — use the main window, or a
Winamp skin, for those. Resizable skins open at the size they declare.
