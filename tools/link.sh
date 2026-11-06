#!/bin/sh
# Linker helper.
# On a normal Linux system this simply runs the system C compiler driver (`cc`), exactly like cargo
# would by default. Only when no `cc` is installed (for example inside a minimal Flatpak sandbox)
# does it link with the `rust-lld` that ships with Rust, against the system's shared C library.

if command -v cc >/dev/null 2>&1; then exec cc "$@"; fi
if command -v gcc >/dev/null 2>&1; then exec gcc "$@"; fi
if command -v clang >/dev/null 2>&1; then exec clang "$@"; fi

set -e

# ---- find rust-lld ----
LLD=""
for a in "$@"; do
  case "$a" in
    -B*) d="${a#-B}"; [ -x "$d/../rust-lld" ] && LLD="$d/../rust-lld" ;;
  esac
done
if [ -z "$LLD" ]; then
  SYSROOT="$(rustc --print sysroot)"
  LLD="$(ls "$SYSROOT"/lib/rustlib/*/bin/rust-lld | head -n 1)"
fi

# ---- find the system C library ----
LIBDIR=""
for d in /usr/lib/x86_64-linux-gnu /usr/lib64 /lib64 /usr/lib /lib; do
  if [ -e "$d/libc.so.6" ]; then LIBDIR="$d"; break; fi
done
[ -n "$LIBDIR" ] || { echo "link.sh: no libc.so.6 found" >&2; exit 1; }
DYNLINKER=/lib64/ld-linux-x86-64.so.2

# ---- a private folder with plain library names and the start-up code ----
ROOT="${TMPDIR:-/tmp}/pixel-easel-linkroot-$(id -u)"
mkdir -p "$ROOT"
for l in c m gcc_s pthread dl rt util; do
  if [ ! -e "$ROOT/lib$l.so" ]; then
    f="$(ls "$LIBDIR"/lib$l.so.* 2>/dev/null | head -n 1)"
    [ -n "$f" ] && { ln -sf "$f" "$ROOT/lib$l.so.$$" && mv -f "$ROOT/lib$l.so.$$" "$ROOT/lib$l.so"; }
  fi
done
if [ ! -f "$ROOT/start.o" ] || [ ! -f "$ROOT/dso.o" ]; then
  cat > "$ROOT/dso.rs.$$" <<'EOF'
#![no_std]
core::arch::global_asm!(
    ".section .data.rel.ro,\"aw\"",
    ".p2align 3",
    ".globl __dso_handle",
    ".hidden __dso_handle",
    "__dso_handle:",
    ".quad __dso_handle",
);
EOF
  cat > "$ROOT/start.rs.$$" <<'EOF'
#![no_std]
core::arch::global_asm!(
    ".text",
    ".globl _start",
    ".type _start,@function",
    "_start:",
    "xor ebp, ebp",
    "mov r9, rdx",
    "pop rsi",
    "mov rdx, rsp",
    "and rsp, -16",
    "push rax",
    "push rsp",
    "xor r8d, r8d",
    "xor ecx, ecx",
    "mov rdi, qword ptr [rip + main@GOTPCREL]",
    "call qword ptr [rip + __libc_start_main@GOTPCREL]",
    "hlt",
    ".section .rodata",
    ".p2align 2",
    ".globl _IO_stdin_used",
    "_IO_stdin_used:",
    ".long 0x20001",
    ".section .note.GNU-stack,\"\",@progbits",
);
EOF
  # build into private names, then move into place (several links may run at once)
  rustc --crate-type=lib --emit=obj -C relocation-model=pic -C panic=abort -O --edition 2021 --crate-name lk_dso -o "$ROOT/dso.o.$$" "$ROOT/dso.rs.$$"
  rustc --crate-type=lib --emit=obj -C relocation-model=pic -C panic=abort -O --edition 2021 --crate-name lk_start -o "$ROOT/start.o.$$" "$ROOT/start.rs.$$"
  mv -f "$ROOT/dso.o.$$" "$ROOT/dso.o"
  mv -f "$ROOT/start.o.$$" "$ROOT/start.o"
  rm -f "$ROOT/dso.rs.$$" "$ROOT/start.rs.$$"
fi

# ---- turn the cc-style arguments into linker arguments ----
expand() {
  for a in "$@"; do
    case "$a" in
      @*) while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "${a#@}" ;;
      *) printf '%s\n' "$a" ;;
    esac
  done
}

ARGS="$ROOT/args.$$"
expand "$@" > "$ARGS.in"
: > "$ARGS"
shared=0
while IFS= read -r a; do
  case "$a" in
    -Wl,*) printf '%s\n' "${a#-Wl,}" | tr ',' '\n' >> "$ARGS" ;;
    -fuse-ld=*|-B*|-m64|-nodefaultlibs|-nostartfiles|-static-libgcc|-fPIC|-fpic) ;;
    -shared) shared=1; printf '%s\n' "$a" >> "$ARGS" ;;
    *) printf '%s\n' "$a" >> "$ARGS" ;;
  esac
done < "$ARGS.in"

{
  printf '%s\n' "-L$ROOT" "-z" "noexecstack" "--eh-frame-hdr" "$ROOT/dso.o"
  if [ "$shared" = 0 ]; then printf '%s\n' "--dynamic-linker=$DYNLINKER" "$ROOT/start.o"; fi
} >> "$ARGS"

rm -f "$ARGS.in"
# hand the arguments over in a response file (one per line, quoted for lld)
sed 's/\\/\\\\/g; s/"/\\"/g; s/^/"/; s/$/"/' "$ARGS" > "$ARGS.rsp"
rm -f "$ARGS"
status=0
"$LLD" -flavor gnu "@$ARGS.rsp" || status=$?
# the program's own file name (the argument after -o)
OUT="$(sed -n '/^"-o"$/{n;p;q}' "$ARGS.rsp" | sed 's/^"//; s/"$//')"
rm -f "$ARGS.rsp"
# Programs linked here would otherwise need this sandbox's very new C library; point their few
# too-new requests at the versions every x86_64 Linux system has (see tools/glibc_compat.py).
if [ "$status" = 0 ] && [ "$shared" = 0 ] && [ -n "$OUT" ] && command -v python3 >/dev/null 2>&1; then
  python3 -I "$(dirname "$0")/glibc_compat.py" "$OUT" "$LIBDIR" || true
fi
exit $status
