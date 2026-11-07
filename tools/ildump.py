"""Research helper (not part of the app): print the compiled .NET instructions (IL) of methods whose
type and method names match regular expressions, from a .NET library. Read-only.

usage: python3 -I ildump.py <library.dll> <type regex> <method regex>
"""
import re
import struct
import sys


def main(path, type_re, meth_re):
    b = open(path, 'rb').read()
    pe = struct.unpack_from('<I', b, 0x3c)[0]
    nsec = struct.unpack_from('<H', b, pe + 6)[0]
    optsz = struct.unpack_from('<H', b, pe + 20)[0]
    opt = pe + 24
    magic = struct.unpack_from('<H', b, opt)[0]
    ddir = opt + (112 if magic == 0x20b else 96)
    secs = []
    so = opt + optsz
    for i in range(nsec):
        vs, va, rs, ro = struct.unpack_from('<IIII', b, so + i * 40 + 8)
        secs.append((va, max(vs, rs), ro))

    def off(rva):
        for va, sz, ro in secs:
            if va <= rva < va + sz:
                return rva - va + ro
        raise ValueError(hex(rva))

    cli = off(struct.unpack_from('<I', b, ddir + 14 * 8)[0])
    md = off(struct.unpack_from('<I', b, cli + 8)[0])
    vlen = struct.unpack_from('<I', b, md + 12)[0]
    p = md + 16 + vlen
    nstreams = struct.unpack_from('<H', b, p + 2)[0]
    p += 4
    streams = {}
    for _ in range(nstreams):
        o, s = struct.unpack_from('<II', b, p)
        p += 8
        e = b.index(b'\0', p)
        name = b[p:e].decode()
        p = (e + 4) & ~3
        streams[name] = (md + o, s)
    tpos = streams.get('#~', streams.get('#-'))[0]
    heapsizes = b[tpos + 6]
    valid = struct.unpack_from('<Q', b, tpos + 8)[0]
    rows = {}
    q = tpos + 24
    for t in range(64):
        if valid >> t & 1:
            rows[t] = struct.unpack_from('<I', b, q)[0]
            q += 4
    strsz = 4 if heapsizes & 1 else 2
    guidsz = 4 if heapsizes & 2 else 2
    blobsz = 4 if heapsizes & 4 else 2

    def idx(t):
        return 4 if rows.get(t, 0) > 0xffff else 2

    def coded(tabs, bits):
        return 2 if max(rows.get(t, 0) for t in tabs) < (1 << (16 - bits)) else 4

    strs = streams['#Strings'][0]

    def getstr(i):
        return b[strs + i:b.index(b'\0', strs + i)].decode('utf-8', 'replace')

    def rd(pos, n):
        return struct.unpack_from('<H' if n == 2 else '<I', b, pos)[0]

    tdor = coded([2, 1, 0x1b], 2)
    layouts = {
        0: [2, strsz, guidsz, guidsz, guidsz],
        1: [coded([0, 0x1a, 0x23, 1], 2), strsz, strsz],
        2: [4, strsz, strsz, tdor, idx(4), idx(6)],
        3: [idx(4)],
        4: [2, strsz, blobsz],
        5: [idx(6)],
        6: [4, 2, 2, strsz, blobsz, idx(8)],
        7: [idx(8)],
        8: [2, 2, strsz],
        9: [idx(2), tdor],
        10: [coded([2, 1, 0x1a, 6, 0x1b], 3), strsz, blobsz],
    }
    pos = q
    tables = {}
    for t in sorted(rows):
        if t > 10:
            break
        lay = layouts[t]
        rs = sum(lay)
        recs = []
        for r in range(rows[t]):
            rp = pos + r * rs
            vals = []
            for n in lay:
                vals.append(rd(rp, n))
                rp += n
            recs.append(vals)
        tables[t] = recs
        pos += rs * rows[t]
    typedefs, methods, memberrefs = tables[2], tables[6], tables.get(10, [])

    def mname(tok):
        t, r = tok >> 24, tok & 0xffffff
        if t == 6:
            return getstr(methods[r - 1][3])
        if t == 10:
            return getstr(memberrefs[r - 1][1])
        return hex(tok)

    owner = {}
    for i, td in enumerate(typedefs):
        end = typedefs[i + 1][5] if i + 1 < len(typedefs) else len(methods) + 1
        for m in range(td[5], end):
            owner[m] = getstr(td[1])
    simple = {0x00: 'nop', 0x02: 'ldarg.0', 0x03: 'ldarg.1', 0x04: 'ldarg.2', 0x05: 'ldarg.3', 0x06: 'ldloc.0',
              0x07: 'ldloc.1', 0x08: 'ldloc.2', 0x09: 'ldloc.3', 0x0a: 'stloc.0', 0x0b: 'stloc.1', 0x0c: 'stloc.2',
              0x0d: 'stloc.3', 0x14: 'ldnull', 0x15: 'ldc.i4.m1', 0x16: 'ldc.i4.0', 0x17: 'ldc.i4.1', 0x18: 'ldc.i4.2',
              0x19: 'ldc.i4.3', 0x1a: 'ldc.i4.4', 0x1b: 'ldc.i4.5', 0x1c: 'ldc.i4.6', 0x1d: 'ldc.i4.7', 0x1e: 'ldc.i4.8',
              0x25: 'dup', 0x26: 'pop', 0x2a: 'ret', 0x58: 'add', 0x59: 'sub', 0x5a: 'mul', 0x5b: 'div', 0x5c: 'div.un',
              0x5d: 'rem', 0x5e: 'rem.un', 0x5f: 'and', 0x60: 'or', 0x61: 'xor', 0x62: 'shl', 0x63: 'shr', 0x64: 'shr.un',
              0x65: 'neg', 0x66: 'not', 0x67: 'conv.i1', 0x68: 'conv.i2', 0x69: 'conv.i4', 0x6a: 'conv.i8', 0x6b: 'conv.r4',
              0x6c: 'conv.r8', 0x6d: 'conv.u4', 0x6e: 'conv.u8', 0x76: 'conv.r.un', 0xd1: 'conv.u2', 0xd2: 'conv.u1',
              0xd3: 'conv.i'}
    br1 = {0x2b: 'br.s', 0x2c: 'brfalse.s', 0x2d: 'brtrue.s', 0x2e: 'beq.s', 0x2f: 'bge.s', 0x30: 'bgt.s', 0x31: 'ble.s',
           0x32: 'blt.s', 0x33: 'bne.un.s', 0x34: 'bge.un.s', 0x35: 'bgt.un.s', 0x36: 'ble.un.s', 0x37: 'blt.un.s',
           0xde: 'leave.s'}
    br4 = {0x38: 'br', 0x39: 'brfalse', 0x3a: 'brtrue', 0x3b: 'beq', 0x3c: 'bge', 0x3d: 'bgt', 0x3e: 'ble', 0x3f: 'blt',
           0x40: 'bne.un', 0x41: 'bge.un', 0x42: 'bgt.un', 0x43: 'ble.un', 0x44: 'blt.un'}
    tok_ops = {0x28: 'call', 0x6f: 'callvirt', 0x73: 'newobj', 0x7b: 'ldfld', 0x7d: 'stfld', 0x7e: 'ldsfld',
               0x80: 'stsfld', 0x8c: 'box', 0x71: 'ldobj', 0x81: 'stobj', 0x8d: 'newarr', 0x7c: 'ldflda', 0xd0: 'ldtoken',
               0x74: 'castclass', 0x75: 'isinst', 0x7f: 'ldsflda', 0xa5: 'unbox.any'}
    for mi, m in enumerate(methods, 1):
        name, tname = getstr(m[3]), owner.get(mi, '?')
        if not (re.search(type_re, tname) and re.search(meth_re, name)) or m[0] == 0:
            continue
        o = off(m[0])
        h = b[o]
        if h & 3 == 2:
            size, code = h >> 2, o + 1
        else:
            size, code = struct.unpack_from('<I', b, o + 4)[0], o + (struct.unpack_from('<H', b, o)[0] >> 12) * 4
        print(f'== {tname}::{name}  token=0x{0x06000000 | mi:08x} rva=0x{m[0]:x}')
        i = 0
        while i < size:
            op = b[code + i]
            at = i
            i += 1
            if op == 0xfe:
                op2 = b[code + i]
                i += 1
                txt = {0x01: 'ceq', 0x02: 'cgt', 0x03: 'cgt.un', 0x04: 'clt', 0x05: 'clt.un'}.get(op2, f'fe{op2:02x}')
                if op2 in (0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e):
                    txt += f' {struct.unpack_from("<H", b, code + i)[0]}'
                    i += 2
                if op2 in (0x15, 0x16):
                    i += 4
            elif op in simple:
                txt = simple[op]
            elif op in (0x0e, 0x10, 0x11, 0x12, 0x13):
                txt = f'{["ldarg.s", "", "starg.s", "ldloc.s", "ldloca.s", "stloc.s"][op - 0x0e]} {b[code + i]}'
                i += 1
            elif op == 0x0f:
                txt = f'ldarga.s {b[code + i]}'
                i += 1
            elif op == 0x1f:
                txt = f'ldc.i4.s {struct.unpack_from("<b", b, code + i)[0]}'
                i += 1
            elif op == 0x20:
                txt = f'ldc.i4 {struct.unpack_from("<i", b, code + i)[0]}'
                i += 4
            elif op == 0x21:
                txt = f'ldc.i8 {struct.unpack_from("<q", b, code + i)[0]}'
                i += 8
            elif op == 0x22:
                txt = f'ldc.r4 {struct.unpack_from("<f", b, code + i)[0]}'
                i += 4
            elif op == 0x23:
                txt = f'ldc.r8 {struct.unpack_from("<d", b, code + i)[0]}'
                i += 8
            elif op in br1:
                txt = f'{br1[op]} IL_{i + 1 + struct.unpack_from("<b", b, code + i)[0]:04x}'
                i += 1
            elif op in br4:
                txt = f'{br4[op]} IL_{i + 4 + struct.unpack_from("<i", b, code + i)[0]:04x}'
                i += 4
            elif op in tok_ops:
                t = struct.unpack_from('<I', b, code + i)[0]
                txt = f'{tok_ops[op]} {mname(t) if op in (0x28, 0x6f, 0x73) else hex(t)}'
                i += 4
            elif op == 0x72:
                t = struct.unpack_from('<I', b, code + i)[0] & 0xffffff
                i += 4
                us = streams['#US'][0] + t
                n, k = b[us], 1
                if n & 0x80:
                    n, k = ((n & 0x3f) << 8) | b[us + 1], 2
                txt = 'ldstr "' + b[us + k:us + k + n - 1].decode('utf-16le', 'replace') + '"'
            elif op == 0x45:
                n = struct.unpack_from('<I', b, code + i)[0]
                txt = f'switch({n})'
                i += 4 + 4 * n
            else:
                txt = f'op{op:02x}'
            print(f'  IL_{at:04x}: {txt}')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], sys.argv[3])
