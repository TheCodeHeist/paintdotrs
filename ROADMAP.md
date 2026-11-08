# Roadmap: beta tester requests

From the beta tester's notes (a long-time user of the original editor), received 2026-11-07.
Status: not started unless marked.

## Clipboard
1. ✅ Done: read the latest copied image from the system clipboard; File > New fills width/height with its size.
2. ✅ Done: paste copied images directly (system clipboard, other apps).
3. ✅ Done: pasted image bigger than the canvas: ask "expand canvas" or "keep current size".
4. ✅ Done: cut/copy/paste of selections works between open pictures and other programs.
5. ✅ Done (uses the system's curl or wget): an image copied from a web page (a link to it): download it and paste at full resolution.

## Editing
6. ✅ Done: treat everything as pixels once deselected; any selection (including text) can be free-transformed
   (move, resize, rotate) with handles. Pasted/dropped/imported pictures arrive with the handles on.
   The canvas only masks: parts pushed past its edge are kept as a smart object, never cut off.
7. ✅ Done: selected pixels stay "floating" and only merge into the layer when deselected (Enter, Deselect,
   another tool); Escape or Undo puts them back.
8. ✅ Done: text can be resized after placing (select it, free transform); font picker with every system
   font family (searchable) and its weights/italics; default Calibri like the original, else Carlito.
9. ✅ Done: Magic Wand checked for speed and accuracy (12-megapixel picture well under a second) and given
   the original's Layer / Image sampling choice.

## Added on request (2026-11-07)
- Every action is final at once (2026-11-08): paste, each move/resize/rotate, each arrow press and each
  stroke (Clone Stamp included) is its own History step immediately; nothing waits for Enter.
- History steps have icons: the tool that made the step, or a symbol for selection, layer, whole-picture,
  effect and file steps.

## Controls
10. ✅ Done: mouse wheel over a number box increases/decreases the value (3 steps per notch, like the original).
11. ✅ Done: Ctrl+wheel zooms smoothly by 1.12× per notch around the pointer, like the original.

## Saving
12. ✅ Done: saving keeps the work as a layered project file (.pxe) and asks where; overwrite only when asked.
13. ✅ Done: exporting to PNG/JPG/... always writes a copy and never overwrites the opened file unless the person
    explicitly chooses to.

## Round 2 (received 2026-11-08), version 0.1.1
1. ✅ Palette colours have a white outline in dark mode and a black one in light mode.
2. ✅ Tools, Colors, History and Layers are floating windows: draggable, resizable, closable, back from View
   (F5–F8); slim title bars.
3. ✅ Dropping or importing a picture bigger than the canvas asks Expand canvas / Keep canvas size.
4. ✅ Faster on big pictures: moving pasted pictures and smart objects redraws only what changed; strokes and
   selections no longer redraw the whole picture when they finish.
5. ✅ Layers can be dragged to a new place in the list.
6. ✅ Autosave every minute to a cache; recovery offered after a crash or power cut.
7. ✅ Shift keeps proportions on the side handles too.
8. ✅ A copied area pastes back in the same place (e.g. on another layer).
9. ✅ Arrow keys nudge with the move tools (Ctrl = 10 px, like the original), otherwise scroll.
10. ✅ Move Selection has free transform handles (move, resize, rotate the selection).
11. ✅ Brush hardness slider in the toolbar.
12. ✅ Auto-Level now works like the original (0.5 % / 99.5 % points and the mean, per channel, selection only).
13. ✅ Adjustment and effect windows can be dragged anywhere.
14. ✅ Font preview ("Lorem ipsum") of the font under the pointer in the font list.
15. ✅ Clicking the font search box no longer closes the font list.
16. ✅ Drop Shadow (Effects > Object) with the original's defaults.
