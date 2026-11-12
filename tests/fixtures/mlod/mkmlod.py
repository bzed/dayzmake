"""Writes small.p3d: a tiny MLOD with the things the old reader lost.

Repeated #Property# and #UVSet# tags, a triangle whose unused 4th vertex is not zero, a quad,
non-zero LOD header flags, an inactive tag. Usage: python3 mkmlod.py small.p3d
"""
import struct, sys

def cs(s): return s.encode() + b'\0'
def tagg(name, data, active=1): return bytes([active]) + cs(name) + struct.pack('<I', len(data)) + data
def prop(k, v): return tagg('#Property#', k.encode().ljust(64, b'\0') + v.encode().ljust(64, b'\0'))
def vert(p, n, u, v): return struct.pack('<IIff', p, n, u, v)

def lod(resolution, faces, nfaces, taggs, flags=0):
    pts = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0)]
    b = b'P3DM' + struct.pack('<IIIIII', 0x1c, 0x100, len(pts), 2, nfaces, flags)
    for p in pts: b += struct.pack('<fffI', *p, 0)
    b += struct.pack('<fff', 0, 0, -1) * 2
    b += faces + b'TAGG' + taggs
    return b + tagg('#EndOfFile#', b'') + struct.pack('<f', resolution)

tri = struct.pack('<I', 3) + vert(0, 0, 0, 0) + vert(1, 0, 1, 0) + vert(2, 0, 0, 1)
tri += vert(7, 7, 0.5, 0.25)  # unused 4th vertex, deliberately not zero
tri += struct.pack('<I', 0x10) + cs('t21\\data\\a_co.paa') + cs('t21\\data\\m.rvmat')
quad = struct.pack('<I', 4) + vert(0, 1, 0, 0) + vert(1, 1, 1, 0) + vert(3, 1, 1, 1) + vert(2, 1, 0, 1)
quad += struct.pack('<I', 0) + cs('#(argb,8,8,3)color(0.5,0.5,0.5,1,CO)') + cs('')
tri2 = tri.replace(b't21\\data\\a_co.paa', b't21\\data\\gone_co.paa')

visual = lod(1.0, tri + quad + tri2, 3, b''.join([
    prop('class', 'house'), prop('autocenter', '0'),
    tagg('#UVSet#', struct.pack('<I', 0) + b'\0' * 24), tagg('#UVSet#', struct.pack('<I', 1) + b'\1' * 24),
    tagg('#Mass#', struct.pack('<4f', 1, 2, 3, 4)),
    tagg('body', bytes(4) + bytes(3)), tagg('old', b'', active=0),
]), flags=0x0a)
memory = lod(1.0e15, b'', 0, tagg('#Property#', b'x'.ljust(64, b'\0') + b'y'.ljust(64, b'\0')) + tagg('start', bytes(4)))

open(sys.argv[1], 'wb').write(b'MLOD' + struct.pack('<II', 0x101, 2) + visual + memory)
