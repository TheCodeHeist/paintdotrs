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
- Undo/Redo step through each move/resize/rotate of a floating picture; the paste only goes away after
  the last undo. These steps show in History (in italics) until the picture is put down.
- History steps have icons: the tool that made the step, or a symbol for selection, layer, whole-picture,
  effect and file steps.

## Controls
10. ✅ Done: mouse wheel over a number box increases/decreases the value (3 steps per notch, like the original).
11. ✅ Done: Ctrl+wheel zooms smoothly by 1.12× per notch around the pointer, like the original.

## Saving
12. ✅ Done: saving keeps the work as a layered project file (.pxe) and asks where; overwrite only when asked.
13. ✅ Done: exporting to PNG/JPG/... always writes a copy and never overwrites the opened file unless the person
    explicitly chooses to.
