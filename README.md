# Paint.RS

A layered image editor for Linux.

## Install a package (easiest)

The ready-made packages are in the `dist` folder. Double-click the one for your system to open it in
your software installer, or use a terminal:

- Debian, Ubuntu, Linux Mint, Pop!_OS:
  `sudo apt install ./dist/paint-rs_0.1.1_amd64.deb`
- Fedora, RHEL, AlmaLinux, Rocky:
  `sudo dnf install ./dist/paint-rs-0.1.1-1.x86_64.rpm`
- openSUSE:
  `sudo zypper install ./dist/paint-rs-0.1.1-1.x86_64.rpm`

Then open **Paint.RS** from your applications menu, or type `paint-rs` in a terminal.
You can also pass pictures to open: `paint-rs photo.png drawing.pxe`.

Needs a 64-bit PC with Linux from 2021 or newer (glibc 2.34+: Ubuntu 22.04, Debian 12, Fedora 35,
RHEL 9, or later).

To remove it: `sudo apt remove paint-rs` or `sudo dnf remove paint-rs`.

## Drag and drop

Drop picture files, or pictures dragged straight from a web browser, onto the window. One picture opens right
away; with several, or when a picture is already open, Paint.RS asks whether to open them or add them as
layers. Hold Shift while dropping to add them as layers without being asked. Works on Wayland and X11
(`PAINT_RS_X11=1 paint-rs` forces X11 mode).

## Saving

- **Save** (Ctrl+S) keeps your work as a layered project file (`.pxe`). A PNG or JPG you opened is never
  overwritten by Save.
- **Export a Copy** (Ctrl+E) writes a flat PNG, JPG, WebP… as a new file. It asks before replacing any file.

## Copy and paste

Copy and paste work between pictures, windows and other programs. Copying a picture's web address also
works: it's downloaded (needs `curl` or `wget`, normally installed) and pasted at full size.

## Free transform and smart objects

A pasted, dropped or imported picture arrives floating, with handles: drag to move it, drag the square
handles to resize (hold Shift to keep its proportions), drag the round handle or right-drag to rotate.
The Move Selected Pixels tool (M) does the same for any selection, text included, and the Move Selection
tool transforms the selection outline itself. Pasting something you copied in Paint.RS puts it back in the
same place, so a copied area lands exactly over the original on another layer.

It turns into ordinary pixels when you deselect (Ctrl+D), press Enter or Escape, or pick another tool, like the
original. Every action is final the moment you do it: the paste, each move, resize, rotation, arrow press and
brush stroke is its own History step at once (nothing waits for Enter), and Undo takes them back one by one. The canvas only masks your work: whatever you push past its edge is kept as a **smart object** layer
(marked "◆ smart") so it reappears when you move it back or enlarge the canvas. Smart objects are saved in
`.pxe` projects; to paint on one, use Layers > Rasterize Layer.

## Windows and keys

Tools, Colors, History and Layers are small floating windows: drag them anywhere, resize them, close them
with their ×, and bring them back from the View menu (Tools F5, History F6, Layers F7, Colors F8).
Drag a layer up or down in the Layers window to change its order. Arrow keys nudge the selection or the
floating picture with the move tools (Ctrl: 10 pixels); a run of presses is one undo step.

## Never lose work

Pictures with unsaved changes are backed up every minute. If the computer crashes or loses power, the next
start offers to recover them.

## Run without installing

    ./target/release/pixel_easel

## Build it yourself

You need Rust (https://rustup.rs). In this folder:

    cargo build --release        # the program: target/release/pixel_easel
    ./package.sh                 # also makes the .deb and .rpm in dist/

The packages are unsigned, so some installers ask you to confirm before installing them.
