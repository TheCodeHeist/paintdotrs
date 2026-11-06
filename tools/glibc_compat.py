"""Make a freshly linked program run on older Linux systems.

Used only by tools/link.sh when it links with Rust's own linker inside a sandbox whose C library is
newer than most distributions. Some maths functions (atan2f, hypot, ...) got new symbol versions in
recent glibc; the same functions also exist under the original GLIBC_2.2.5 version on every x86_64
system. This rewrites each too-new version request to GLIBC_2.2.5, after checking in the real library
that every symbol using it really exists at GLIBC_2.2.5 (or is optional/weak). Anything that can't be
redirected safely makes it stop with an explanation instead of guessing.

usage: python3 glibc_compat.py <program> <system library folder>
"""
import os
import struct
import sys

TARGET = "GLIBC_2.2.5"
MAX_MINOR = 34  # keep version requests at or below glibc 2.34 (Ubuntu 22.04, Debian 12, RHEL 9 and newer)


def elf_hash(name: bytes) -> int:
    h = 0
    for c in name:
        h = ((h << 4) + c) & 0xFFFFFFFF
        g = h & 0xF0000000
        if g:
            h ^= g >> 24
        h &= ~g & 0xFFFFFFFF
    return h


def sections(b):
    shoff = struct.unpack_from("<Q", b, 0x28)[0]
    es, n, _ = struct.unpack_from("<HHH", b, 0x3A)
    return [struct.unpack_from("<IIQQQQIIQQ", b, shoff + i * es) for i in range(n)]


def cstr(b, off):
    return b[off:b.index(b"\0", off)]


def defined_versions(lib_path):
    """{symbol: set(version names)} for symbols a shared library defines."""
    b = open(lib_path, "rb").read()
    secs = sections(b)
    dynsym = next(s for s in secs if s[1] == 11)
    strtab = secs[dynsym[6]]
    versym = next(s for s in secs if s[1] == 0x6FFFFFFF)
    verdef = next(s for s in secs if s[1] == 0x6FFFFFFD)
    names = {}
    off = verdef[4]
    while True:
        _v, _f, ndx, cnt, _h, aux, nxt = struct.unpack_from("<HHHHIII", b, off)
        if cnt:
            nm = struct.unpack_from("<I", b, off + aux)[0]
            names[ndx] = cstr(b, strtab[4] + nm).decode()
        if not nxt:
            break
        off += nxt
    out = {}
    for i in range(dynsym[5] // 24):
        nm, _info, _other, shndx, _val, _size = struct.unpack_from("<IBBHQQ", b, dynsym[4] + i * 24)
        if shndx == 0:
            continue
        v = struct.unpack_from("<H", b, versym[4] + i * 2)[0] & 0x7FFF
        if v in names:
            out.setdefault(cstr(b, strtab[4] + nm).decode(), set()).add(names[v])
    return out


def main(prog, libdir):
    b = bytearray(open(prog, "rb").read())
    secs = sections(b)
    dynsym = next((s for s in secs if s[1] == 11), None)
    verneed = next((s for s in secs if s[1] == 0x6FFFFFFE), None)
    versym = next((s for s in secs if s[1] == 0x6FFFFFFF), None)
    if not (dynsym and verneed and versym):
        return 0
    dynstr = secs[dynsym[6]]
    strs = bytes(b[dynstr[4]:dynstr[4] + dynstr[5]])
    target_off = strs.find(b"\0" + TARGET.encode() + b"\0")
    if target_off < 0:
        print("glibc_compat: no GLIBC_2.2.5 string to point at; leaving the program as it is", file=sys.stderr)
        return 0
    target_off += 1

    # imported symbols by version index
    users = {}
    for i in range(dynsym[5] // 24):
        nm, info, _o, shndx, _v, _s = struct.unpack_from("<IBBHQQ", b, dynsym[4] + i * 24)
        if shndx != 0:
            continue
        v = struct.unpack_from("<H", b, versym[4] + i * 2)[0] & 0x7FFF
        users.setdefault(v, []).append((cstr(b, dynstr[4] + nm).decode(), info >> 4 == 2))

    changed = []
    off = verneed[4]
    while True:
        _ver, cnt, filen, aux, nxt = struct.unpack_from("<HHIII", b, off)
        lib = cstr(b, dynstr[4] + filen).decode()
        a = off + aux
        lib_defs = None
        for _ in range(cnt):
            _h, _flags, other, name, anext = struct.unpack_from("<IHHII", b, a)
            vname = cstr(b, dynstr[4] + name).decode()
            if vname.startswith("GLIBC_2.") and vname.count(".") == 1 and int(vname.split(".")[1]) > MAX_MINOR:
                if lib_defs is None:
                    path = os.path.join(libdir, lib)
                    lib_defs = defined_versions(path) if os.path.exists(path) else {}
                for sym, weak in users.get(other, []):
                    if not weak and TARGET not in lib_defs.get(sym, set()):
                        print(f"glibc_compat: {sym}@{vname} from {lib} has no {TARGET} version; "
                              "the program will need glibc 2.{} or newer".format(vname.split('.')[1]), file=sys.stderr)
                        break
                else:
                    struct.pack_into("<I", b, a, elf_hash(TARGET.encode()))
                    struct.pack_into("<I", b, a + 8, target_off)
                    changed.append(f"{lib}:{vname}")
            if not anext:
                break
            a += anext
        if not nxt:
            break
        off += nxt
    if changed:
        tmp = prog + ".compat"
        with open(tmp, "wb") as f:
            f.write(b)
        os.chmod(tmp, os.stat(prog).st_mode)
        os.replace(tmp, prog)
        print("glibc_compat: now asks for GLIBC_2.2.5 instead of " + ", ".join(changed), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
