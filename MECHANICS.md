# Mechanics sheet

What the original image editor does, read from its own code, and what this rebuild uses.
Addresses are native addresses (paintdotnet.exe) or .NET method tokens (`0x06xxxxxx`) in the named
library. **estimated** marks a value that was looked for in the code but not found.

## 1. The main program is a launcher (Ghidra decompile of paintdotnet.exe)
- `entry` 0x140001c18 → CRT start-up 0x1400019c0 → `FUN_140001430`.
- `FUN_140001430` (0x140001430): checks the app package id `dotPDNLLC.paint.net`, builds the path of
  `paintdotnet.dll` next to the exe (`FUN_140001020` 0x140001020 = path combine), finds `hostfxr.dll`
  (or asks `nethost.dll` `get_hostfxr_path` in `FUN_140001190` 0x140001190), sets
  `COMPlus_legacyCorruptedStateExceptionsPolicy=1` and `DOTNET_EnableWriteXorExecute=0`, then calls
  `hostfxr_initialize_for_dotnet_command_line` → `hostfxr_run_app` → `hostfxr_close`.
- The exe has 79 functions and no editor logic. Every feature lives in the .NET libraries, so the rules
  below were decoded from their compiled IL (method tokens given).
- The rebuild has no launcher: it is one native Rust program.

## 2. Layer blending: exact
Per-channel rules, `CompositionOps/<Mode>.IBlend.BlendComponent(dst, src)` in PaintDotNet.Core.dll
(`b` = below/dst, `t` = top/src, `FastScale(a,b) = ((a·b+128) + ((a·b+128)>>8)) >> 8`, ByteUtil 0x06000043):

| Mode | Token | Formula |
|---|---|---|
| Multiply | 0x060011f2 | FastScale(b, t) |
| Additive | 0x060011fc | min(255, b + t) |
| Color Burn | 0x06001206 | t = 0 → 0, else max(0, 255 − (u16)((255−b)·255) / t) |
| Color Dodge | 0x06001210 | t = 255 → 255, else min(255, (u16)(b·255) / (255−t)) |
| Reflect | 0x0600121a | t = 255 → 255, else min(255, b² / (255−t)) |
| Glow | 0x06001225 | Reflect with b and t swapped |
| Overlay | 0x0600122f | b < 128 → FastScale(2b, t), else 255 − FastScale(2(255−b), 255−t) |
| Difference | 0x06001239 | abs(t − b) |
| Negation | 0x06001243 | 255 − abs(255 − b − t) |
| Lighten | 0x0600124d | max(b, t) |
| Darken | 0x06001257 | min(b, t) |
| Screen | 0x06001261 | t + b − FastScale(t, b) |
| Xor | 0x0600126b | b xor t |

Mixing with transparency, `CompositionOps.OpImpl<T>.Apply` (0x06001277):
- top alpha 0 → keep below.
- belowOnly = FastScale(belowA, 255 − topA); total = belowOnly + topA; total 0 → transparent.
- overlap = FastScale(belowA, topA); topOnly = topA − overlap.
- colour = (below·belowOnly + top·topOnly + blended·overlap) / total; alpha = total.
- Layer opacity scales the top alpha first (FastScale(topA, opacity)). This step is derived from the
  `…WithOpacity` op type names; its body was not read.
- Blend modes listed in the string table `LayerBlendMode.*`.

## 3. Grey value (intensity): exact
`ColorBgra.GetIntensityByte` (PaintDotNet.Core.dll 0x06000132): `(7471·B + 38470·G + 19595·R) >> 16`.
Black and White = `UnaryPixelOps.Desaturate.Apply` (0x06001139): R = G = B = intensity, alpha kept.

## 4. Brightness / Contrast: exact
`BrightnessAndContrastAdjustment` (PaintDotNet.Effects.Legacy.dll, OnCreatePropertyCollection 0x06000012,
OnSetRenderInfo 0x06000014, OnRender 0x06000015).
- Brightness and Contrast: −100…100, default 0.
- contrast < 0: mul = contrast + 100, div = 100; contrast > 0: mul = 100, div = 100 − contrast; else 1/1.
- Table `[intensity·256 + channel]`: div = 100 → shift = (i−127)·mul/div + 127 − i + brightness;
  otherwise shift = (i − 127 + brightness)·mul/div + 127 − i; out = clamp(channel + shift).
- div = 0 (contrast 100): grey = 0 if intensity + brightness < 128, else 255.

## 5. Hue / Saturation: exact
Ranges, `HueAndSaturationAdjustment.OnCreatePropertyCollection` (Effects.Legacy 0x060000fd):
Hue −180…180 (0), Saturation 0…200 (100), Lightness −100…100 (0).
`OnSetRenderInfo` (0x060000fe): saturation above 100 becomes (s − 100)·3 + 100.
`UnaryPixelOps.HueSaturationLightness` (Core .ctor 0x0600113d, Apply 0x06001140, table 0x0600113f):
- hueDelta = hue·6/360 (in sixths of the colour wheel); satFactor = sat·65536/100.
- Saturation: c = clamp(((I << 16) + (c − I)·satFactor) >> 16), I = intensity.
- Hue: hue from max/min of R,G,B (R max: (G−B)/d; G max: (B−R)/d + 2; else (R−G)/d + 4), add hueDelta,
  wrap to 0…6, rebuild keeping max and min.
- Lightness table: t[i] = (byte)(0.5 + 255·lerp(i/255, L > 0 ? 1 : 0, |L|/100)).

## 6. Posterize: exact
`PosterizeAdjustment.OnCreatePropertyCollection` (Effects.Legacy 0x06000201): Red, Green, Blue 2…64,
default 16, plus a Linked switch (default on).
`PosterizePixelOp.CalcLevels` (0x060002a6): levels[0] = 0, levels[i] = 255·i/(n−1); the 256-entry table
walks the levels: t[v] = levels[j]; acc += n; if acc > 255 { acc −= 255; j += 1 }.

## 7. Sepia: exact
`SepiaEffect.OnSetRenderInfo` (Effects.Legacy 0x0600022e): Desaturate, then
`UnaryPixelOps.Level(black, white, gamma {1.2, 1.0, 0.8}, black, white)`. The float array is stored in the
file at offset 0x61330. Gamma order is B, G, R.
`Level.Apply` (Core 0x06001161): out = (byte)clamp(outLo + (outHi − outLo)·((v − inLo)/(inHi − inLo))^gamma).
So blue = 255·(g/255)^1.2, green = g, red = 255·(g/255)^0.8.
The Levels adjustment in this rebuild uses the same Level formula with one gamma for all channels.

## 8. Magic Wand / Paint Bucket / Recolor tolerance: exact
- `ToolsSection` .ctor (paintdotnet.dll 0x06002adf): Tolerance default **0.5** (range 0…1);
  ToleranceAlphaMode default value 1 = `Straight` (enum order Premultiplied, Straight).
- `ToleranceUtil.ConvertToByte` (0x06001b98): t = round(v·255), tolByte = FastScale(t, t).
- `CheckColorStraightFunctor.Invoke` (0x0600325f): match when distance ≤ tolByte.
- `FloodFillAlgorithm.GetDistanceStraight` (0x0600200d) + `GetDistanceEuclidean` (0x0600200e):
  identical colours → 0; otherwise max(1, round(0.5 · 255 · |a − b|)), using RGBA scaled to 0…1.
- Flood modes Contiguous / Global (string table `FloodMode.*`).

## 9. Tool defaults: exact
From `AppSettings` sections in paintdotnet.dll:
- `ToolsSection` .ctor 0x06002adf: Antialiasing on; Primary = black, Secondary = white.
- `BrushSection` .ctor 0x06003574: Size **2** (1…2000), Hardness 0.75 (0…1), Spacing **0.15** of size
  (0.01…5). The rebuild uses all three: solid inside 0.75 of the radius, then a linear fade to the edge
  (the fade shape is **estimated**).
- `TextSection` .ctor 0x06003575: FontSize **12** (1…2000). The default font is Calibri; the rebuild uses its
  built-in Ubuntu Light (Calibri is not freely shareable).
- `ShapesSection` .ctor 0x06003579: Radius **10** (0…2000) for rounded rectangles.

## 10. Zoom: exact
`ScaleFactor` static table (paintdotnet.dll .cctor 0x060001c2), 45 steps:
1/100, 1/88, 1/76, 1/64, 1/56, 1/48, 1/40, 1/32, 1/28, 1/24, 1/20, 1/16, 1/14, 1/12, 1/10, 1/8, 1/6, 1/5,
1/4, 1/3, 1/2, 2/3, 1, 3/2, 2, 3, 4, 5, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 40, 48, 56, 64, 76, 88, 100
(1 % … 10000 %).

## 11. Tools and menus (string table `*Tool.Name`, `*Tool.HelpText`, `Menu.*`)
21 tools: Move Selected Pixels, Move Selection, Rectangle / Lasso / Ellipse Select, Magic Wand, Zoom,
Pan, Paint Bucket, Gradient, Paintbrush, Eraser, Pencil, Color Picker, Clone Stamp, Recolor, Text, Line,
Rectangle, Rounded Rectangle, Ellipse.
- Left button = primary colour, right button = secondary. Shift constrains shapes and selections and
  switches the wand and bucket to Global. Ctrl+click sets the Clone Stamp source. Ctrl on the Color Picker
  samples all layers. The Pencil is one pixel wide.
- New layer name "Background" (`Layer.BackgroundLayer.DefaultName`), new image "Untitled".
- New image size: `AppWorkspace.GetNewDocumentSize` (paintdotnet.dll 0x06001421): 800 × 600 when the screen is
  landscape, 600 × 800 when portrait (aspect < 1.0), converted from DIPs to pixels by the UI scale.
  (The very first blank image at start-up uses 800 × 600.)
- Shift angle snap: 15° steps (angle·12/π rounded away from zero), `GradientToolChanges.ConstrainPoints`
  (paintdotnet.dll 0x06001fd4). Lines (`ShapesToolChanges.ConstrainPoints15Degrees` 0x06001d8f) also round the
  length to whole pixels at 0°/90° and to steps of √2 at 45°.
- Still **estimated**: gradient type set, selection mode
  set, other effects' formulas (blurs, distortions, noise, sharpen, vignette,
  clouds, emboss, edge detect, auto-level).

## Mouse wheel: exact
- Canvas, `CanvasControl.ProcessVerticalPointerWheel` (paintdotnet.dll 0x06002299): no key = scroll up/down,
  Shift = scroll sideways, by delta / zoom canvas pixels (one notch = 120 → 120 screen pixels); Ctrl = zoom
  around the pointer by 1.12^(|delta|/120) (in for wheel up, out for down), clamped to the zoom range, then
  snapped to round(zoom) when |round − new| < 5 % of round and closer than the old zoom. Other keys: nothing.
- Number boxes, `PdnNumericSpin.OnMouseWheel` (PaintDotNet.Framework.dll 0x0600077f): wheel delta is
  accumulated; each notch presses Up/Down "lines per notch" times (system setting, 3 by default); ignored
  while Shift or Alt is held.
- Touchpads report pixels, not notches: 50 pixels count as one notch (**estimated**).

## Paste: exact
`PasteImageAction.PerformActionImpl` (paintdotnet.dll 0x060028f4), `ShowExpandCanvasTaskDialog` (0x060028f7):
- No picture on the clipboard → "The clipboard doesn't contain an image." (string table `PasteAction.Error.NoImage`).
- Picture wider or taller than the canvas → ask: Expand canvas (new size = max of both, per side) / Keep
  canvas size / Cancel (`ExpandCanvasQuestion.*`).
- Placement: at the canvas's top-left if it fits inside the visible part there; otherwise moved to start at
  the visible part's left/top edge, or to end at its right/bottom edge if it would stick out; never left of or
  above the canvas. The pasted pixels become the selection and the Move Selected Pixels tool is chosen.

## Drag and drop
- Original, `MainForm.OnDragDropImpl` (paintdotnet.dll 0x06001146) and `OnDragEnter` (0x06001143): only files
  (folders are skipped). One file with no picture open opens directly; otherwise it asks "Open" (each as its
  own picture) / "Add layers" (to the current picture) / Cancel (`DragDrop.OpenOrImport.*` strings).
- The rebuild does the same: folders skipped; one picture with nothing open opens directly; otherwise it asks
  Open / Add layers / Cancel. Additions: pictures dragged from web browsers (http/https/data: addresses, or
  raw picture data), and Shift+drop adds them as layers without asking.
- Edit menu keys, from `EditMenu..ctor` (paintdotnet.dll 0x06000abc), in menu order: Undo Ctrl+Z, Redo Ctrl+Y,
  Cut Ctrl+X, Copy Ctrl+C, Copy Merged Ctrl+Shift+C, Paste Ctrl+V, Paste into New Layer Ctrl+Shift+V, Paste into
  New Image Ctrl+Alt+V, Copy Selection Ctrl+Shift+Alt+C, Paste Selection Ctrl+Shift+Alt+V, Erase Selection Delete,
  Invert Selection Ctrl+I, Select All Ctrl+A, Deselect Ctrl+D. All used as-is. Copy/Paste Selection copy and
  re-apply only the selection outline (kept inside the app).

## Free transform and smart objects (the person's design)
- Original: `MoveTool.HelpText` (string table) — "Drag the selection to move. Drag the handles to resize. Drag
  with right mouse button to rotate."; `MoveSelectionTool.HelpText` — Shift constrains while resizing or
  rotating. The rebuild follows that: handles resize, right-drag (or the round handle) rotates, Shift keeps
  proportions on corners and snaps rotation to 15° (same step as the line/gradient snap, 0x06001fd4).
- Selected pixels float until deselected (Enter, Deselect, another tool or edit); Escape/Undo restore them.
- Pasted, dropped and imported pictures arrive floating with handles (the person's request).
- The canvas is only a mask (the person's request): when floating pixels are put down and part of them lies
  outside the canvas, they become a smart object layer that keeps the whole source picture and its placement
  (centre, scale, rotation); Canvas Size, Crop, Resize, Flip and Rotate move the canvas over it without
  cutting anything. Smart objects are stored only in `.pxe` (format version PXEASEL2); exports are flattened.
- Resampling, from the original: `MoveToolContentRenderer.GetEffectiveInterpolationMode` (paintdotnet.dll
  0x06001ed1) uses nearest-neighbour (an exact copy) for whole-pixel moves and the chosen mode otherwise; the
  default mode is `ToolsSection` "MoveToolInterpolationMode" = 5 (0x06002adf) = High Quality Cubic (enum
  NearestNeighbor, Linear, MultisampleLinear, Anisotropic, HighQualityCubic, numbered from 1 — derived from
  5 being the default). The rebuild: exact copy for whole-pixel moves; Catmull-Rom cubic in premultiplied
  alpha (clamped against ringing) when enlarging or rotating; an averaged grid of samples when shrinking
  (**estimated** stand-in for the original's cubic minification).

## Text tool font
- `TextSection` .ctor (paintdotnet.dll 0x06003575): FontFamilyName default "Calibri", FontSize 12. The rebuild
  lists every system font (searchable) with its weights and italics; default Calibri when installed, else
  Carlito (same letter widths), Ubuntu, Noto Sans, Cantarell, DejaVu Sans, Liberation Sans (**estimated** order).
  Size unit: `TextSection` "FontSizeMetric" default 0 = Points (enum Points, Fixed96Dpi); `TextTool.
  GetSizedFontProperties` (paintdotnet.dll 0x06001cf4) sizes the font from the picture's resolution. The
  rebuild uses points at the original's default 96 DPI: pixels = points × 96 / 72 (12 pt = 16 px), range
  1–2000 pt like the original.

## Magic Wand / Paint Bucket sampling
- `ToolsSection` "SampleAllLayers" (0x06002adf), default off: compare colours on the active layer; on =
  the whole picture. Rebuilt as the Sampling: Layer / Image choice.

## History (undo)
- `HistoryStack.PushNewMemento` (paintdotnet.dll 0x0600010e): a new step clears the redo steps, is added to the
  stack and the stack is flushed (old steps go to disk); there is no step-count limit.
- The rebuild keeps every step as well. Unchanged layers and selections are shared between steps, so a step only
  stores what it changed. Only if the steps' own data would pass 1.5 GB are the oldest dropped (that safety
  budget is **estimated**; the original uses disk instead).

## Auto-Level: exact (version 0.1.1)
`AutoLevelEffect.OnSetRenderInfo` (PaintDotNet.Effects.Legacy.dll 0x0600000c) builds an RGB histogram of
the **selected** pixels, then `HistogramRgbLegacy.MakeLevelsAuto` (0x060000f9) calls
`LevelOpLegacy.AutoFromLoMidHi(lo, mid, hi)` (0x0600012d) with:
- lo = `GetPercentileColor(0.005)`, hi = `GetPercentileColor(0.995)`: per channel the first bin where the
  running count exceeds total × fraction (`HistogramLegacy.GetPercentile` 0x060000ec);
- mid = `GetMeanColor()`: the channel mean + 0.5, truncated (0x060000f5).
- Per channel: gamma = clamp(Math.Log(0.5, (mid − lo)/(hi − lo)), 0.1, 10) when lo < mid < hi, else 1;
  the channel then maps lo → black, hi → white with that gamma (same Level op as Levels).
- Our addition: fully transparent pixels are left out of the histogram (they have no visible colour).
  The earlier rebuild stretched plain min/max, which on most photos changed nothing.

## Drop Shadow (Effects > Object): defaults exact
`DropShadowGpuEffect.OnCreatePropertyCollection` (PaintDotNet.Effects.Gpu.dll 0x060002b6), property order
confirmed by `OnCreateConfigUI` (0x060002b7) display names: Blur radius 10 (0..100), Distance 10 (0..100),
Angle −45° (−180..180), Opacity 0.75 (0..1), Colour black, Shadow only off. The original notes it needs a
layer with transparency. Rebuild: the layer's alpha shifted by distance toward the angle (counter-clockwise
from the right, so −45° is down-right), Gaussian-blurred, drawn in black at the opacity behind the layer.
The colour is fixed to the default black (the colour choice is not offered yet).

## Tester round 2 additions (version 0.1.1, the person's design)
- Tools, Colors, History and Layers are floating, resizable windows with slim headers; close them and bring
  them back from View (Tools F5, History F6, Layers F7, Colors F8 — the original's keys) or reset positions.
- Layers can be dragged up and down the list to change their order.
- Pasting our own copy puts it back where it was copied from (any layer or picture of the same size).
- A dropped or imported picture bigger than the canvas asks Expand canvas / Keep canvas size, like pastes.
- Move Selection has the same free transform handles as Move Selected Pixels (move, resize, rotate); hard
  selections stay hard-edged.
- Arrow keys: see "Move tool keys" below (decoded); other tools scroll the view.
- Shift keeps proportions on the side handles too (the other side grows by the same factor).
- Brush hardness is a toolbar setting (default 75 %, the original's BrushSection value).
- Unsaved pictures are backed up every 60 s to `~/.cache/paint-rs/autosave` (**estimated** interval);
  after a crash or power cut the next start offers to recover them.
- Speed: moving a floating picture or a smart object redraws only the old and new boxes; finished strokes
  and selections no longer force a full redraw of the picture.

## Move tool keys: exact (Ghidra pass of paintdotnet.exe again: FUN_140001430 0x140001430 only starts paintdotnet.dll, so these come from its IL)
- `TransformControl.ProcessArrowKey` (paintdotnet.dll 0x060020e6): Left/Up/Right/Down (WPF keys 23–26) move
  by (−1,0)/(0,−1)/(1,0)/(0,1) pixels; with **Ctrl** (`ModifierKeys & 2`) the step is ×10. (The earlier
  rebuild used Shift — wrong.)
- `TransformControl.OnKeyDown` (0x060020e5): the first arrow press starts one edit; each press restarts a
  timer of max(500 ms, KeyboardDelayMs × 3 / 2); when it runs out the edit is committed, so a run of presses
  is one history step. Escape during a mouse drag cancels that drag. **Rebuild differs on purpose (the
  person's rule, 2026-11-08): every action is final at once** — each arrow press, each finished drag of a
  floating picture, the paste itself and every brush/Clone Stamp stroke is its own history step
  immediately; nothing waits for a pause, Enter or deselecting. Putting floating pixels down adds a
  "Finish Pixels" step (the original's name for it).
- `MoveTool.OnUIPreviewKeyDown` (0x06001e9c): Enter (key 6) or Escape (key 13) with a selection ends any
  drawing/editing, commits the moved pixels and deselects. Rebuild does the same for Move Selected Pixels;
  Undo still takes it back.

## Clone Stamp: exact (paintdotnet.exe pass: FUN_140001430 0x140001430 and FUN_140001190 0x140001190 only find and start paintdotnet.dll, so the rules come from its IL)
- `CloneStampTool.OnUIDragBegin` (paintdotnet.dll 0x06002155): Ctrl (`Modifiers & 2`) + press sets the
  source point and remembers the **active layer** as the source layer (anchor mode 1); Ctrl+drag keeps moving
  it (`OnUIDragMove` 0x06002156, `OnUIDragEnd` 0x06002157). Without a source (mode 0) a stroke does nothing.
- On the first stroke after choosing a source, the source becomes a fixed offset = source − brush point
  (mode 2); **every later stroke keeps that offset** (aligned cloning), until Ctrl+click picks a new source.
  The earlier rebuild re-measured from the source point at each stroke — wrong.
- `GetAnchorLayerIndex` (0x0600215b): pixels are copied from the source layer, even when painting on another.
- `UpdateAnchor` (0x0600214e): the source marker follows the pointer by the offset and has radius = brush
  width / 2.
- Placing the source changes no pixels and adds no history step; each stroke is one step.

## 12. Files
Opens PNG, JPEG, BMP, GIF, TIFF, TGA, WebP, ICO, QOI and its own layered `.pxe`; saves the same except ICO.
The original's `.pdn` format is not read or written.
