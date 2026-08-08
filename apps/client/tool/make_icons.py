#!/usr/bin/env python3
"""Turn `assets/logo.svg` into the source images `icons_launcher` needs.

    python3 tool/make_icons.py && dart run icons_launcher:create

`icons_launcher` is raster-only — it reads PNGs and knows nothing about SVG —
so the logo has to be rendered first, and rendered three different ways,
because the platforms want genuinely different images:

    storm_icon.png        macOS, Linux, web        rounded, with the margin
                                                   macOS reserves around an app
                                                   icon, transparent outside it
    storm_icon_full.png   Android legacy, maskable full bleed; the launcher
                                                   supplies its own mask
    storm_mark.png        Android adaptive         the mark alone, transparent,
                                                   inside the 66% safe zone

## Why it is written this way

This Mac has no SVG rasteriser — no rsvg-convert, inkscape, magick or cairosvg
— and installing one is a poor thing to require of whoever regenerates an icon.
The only renderer macOS ships is QuickLook (`qlmanage`), and it has one
limitation that shapes everything below: **it flattens transparency onto
white.** So the two images that need an alpha channel cannot come out of it
directly.

They are recovered afterwards instead, in stdlib Python:

  - the rounded corners and margin are a mask computed analytically and applied
    to a full-bleed render (`_rounded_mask`), which is more accurate than
    asking the renderer for them;
  - the mark's transparency is un-mixed from a render of dark-on-white
    (`_key_white`). That is exact rather than a chroma-key guess: the mark is a
    single flat colour, so for every pixel there is only one alpha that could
    have produced it.

The trade is a hard dependency on macOS for regenerating icons, which is
already where the client is developed. If a real rasteriser ever gets
installed, `_render` is the only function that needs to change.
"""

import re
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

CLIENT = Path(__file__).resolve().parent.parent
LOGO = CLIENT / 'assets' / 'logo.svg'
OUT = CLIENT / 'assets' / 'icon'

SIZE = 1024
CARD = '#96f2d7'  # the logo's mint card
MARK = '#343a40'  # the tornado

# Apple's macOS icon grid: the rounded square is 824pt of a 1024pt canvas, with
# a corner radius of 185.4. Matching it is what makes the icon sit correctly
# among the others in the Dock rather than looking a size too big.
MAC_INSET = 100
MAC_RADIUS = 185.4

# How much of the card the tornado covers. 0.62 is what it is in the logo
# itself; the Android numbers are smaller because a launcher crops.
FILL_ICON = 0.62
FILL_FULL = 0.55
FILL_MARK = 0.52  # adaptive icons show the middle ~66%, so stay well inside it


# ---- reading the logo ------------------------------------------------------

def mark_groups(svg: str) -> list[str]:
    """The `<g>` elements that draw the tornado — everything but the card."""
    groups = [g for g in re.findall(r'<g\b.*?</g>', svg, re.S) if MARK in g]
    if not groups:
        sys.exit(f'no {MARK} paths in {LOGO} — has the logo changed?')
    return groups


def bounds(groups: list[str]) -> tuple[float, float, float, float]:
    """The tornado's bounding box in the logo's own coordinates.

    Every number in an Excalidraw path is an absolute `x,y` pair in the group's
    local space, so the box is the extent of those pairs plus the group's
    translate. Quadratic control points push it out slightly beyond the curve,
    which errs towards more margin — the safe direction for an icon.
    """
    xs, ys = [], []
    for g in groups:
        tx, ty = (float(n) for n in
                  re.search(r'translate\(([-\d.]+)[ ,]+([-\d.]+)\)', g).groups())
        for d in re.findall(r'\bd="([^"]+)"', g):
            for x, y in re.findall(r'(-?[\d.]+),(-?[\d.]+)', d):
                xs.append(tx + float(x))
                ys.append(ty + float(y))
    return min(xs), min(ys), max(xs), max(ys)


def place(groups: list[str], fill: float) -> str:
    """The tornado, scaled to `fill` of the canvas and centred on it."""
    x0, y0, x1, y1 = bounds(groups)
    scale = SIZE * fill / max(x1 - x0, y1 - y0)
    dx = SIZE / 2 - scale * (x0 + x1) / 2
    dy = SIZE / 2 - scale * (y0 + y1) / 2
    return (f'<g transform="translate({dx:.3f} {dy:.3f}) scale({scale:.5f})">'
            + ''.join(groups) + '</g>')


def canvas(background: str, body: str) -> str:
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{SIZE}" '
            f'height="{SIZE}" viewBox="0 0 {SIZE} {SIZE}">'
            f'<rect width="{SIZE}" height="{SIZE}" fill="{background}"/>'
            f'{body}</svg>')


# ---- rendering -------------------------------------------------------------

def _render(svg: str) -> tuple[int, int, int, bytes]:
    """Rasterise one SVG at SIZE×SIZE. Opaque — see the module docstring."""
    with tempfile.TemporaryDirectory() as tmp:
        src = Path(tmp) / 'icon.svg'
        src.write_text(svg)
        subprocess.run(['qlmanage', '-t', '-s', str(SIZE), '-o', tmp, str(src)],
                       check=True, capture_output=True)
        png = Path(tmp) / 'icon.svg.png'
        if not png.exists():
            sys.exit('qlmanage rendered nothing — is the SVG valid?')
        img = read_png(png.read_bytes())
    w, h, *_ = img
    if (w, h) != (SIZE, SIZE):
        sys.exit(f'qlmanage returned {w}x{h}, expected {SIZE}x{SIZE}')
    return img


# ---- PNG -------------------------------------------------------------------

def read_png(data: bytes) -> tuple[int, int, int, bytearray]:
    """Decode a non-interlaced 8-bit PNG to (w, h, channels, pixels)."""
    assert data[:8] == b'\x89PNG\r\n\x1a\n', 'not a png'
    pos, idat, head = 8, b'', None
    while pos < len(data):
        (ln,) = struct.unpack('>I', data[pos:pos + 4])
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + ln]
        if kind == b'IHDR':
            head = struct.unpack('>IIBBBBB', body)
        elif kind == b'IDAT':
            idat += body
        pos += 12 + ln
    w, h, depth, colour, _, _, interlace = head
    assert depth == 8 and interlace == 0, 'unsupported png variant'
    ch = {0: 1, 2: 3, 4: 2, 6: 4}[colour]
    raw, stride = zlib.decompress(idat), w * ch
    out, prev, p = bytearray(h * stride), bytearray(stride), 0
    for y in range(h):
        f, p = raw[p], p + 1
        line = bytearray(raw[p:p + stride])
        p += stride
        if f:
            for i in range(stride):
                a = line[i - ch] if i >= ch else 0
                b = prev[i]
                c = prev[i - ch] if i >= ch else 0
                if f == 1: line[i] = (line[i] + a) & 255
                elif f == 2: line[i] = (line[i] + b) & 255
                elif f == 3: line[i] = (line[i] + (a + b) // 2) & 255
                else:
                    pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                    pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                    line[i] = (line[i] + pr) & 255
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return w, h, ch, out


def write_png(path: Path, rgba: bytearray) -> None:
    raw = bytearray()
    for y in range(SIZE):
        raw.append(0)  # filter: none
        raw += rgba[y * SIZE * 4:(y + 1) * SIZE * 4]

    def chunk(kind: bytes, body: bytes) -> bytes:
        return (struct.pack('>I', len(body)) + kind + body
                + struct.pack('>I', zlib.crc32(kind + body) & 0xffffffff))

    path.write_bytes(
        b'\x89PNG\r\n\x1a\n'
        + chunk(b'IHDR', struct.pack('>IIBBBBB', SIZE, SIZE, 8, 6, 0, 0, 0))
        + chunk(b'IDAT', zlib.compress(bytes(raw), 9))
        + chunk(b'IEND', b''))


def to_rgba(img: tuple[int, int, int, bytearray]) -> bytearray:
    _, _, ch, data = img
    if ch == 4:
        return data
    out = bytearray(SIZE * SIZE * 4)
    for i in range(SIZE * SIZE):
        p = data[i * ch:i * ch + ch]
        rgb = (p[0], p[0], p[0]) if ch < 3 else (p[0], p[1], p[2])
        out[i * 4:i * 4 + 4] = bytes((*rgb, 255))
    return out


# ---- the two things qlmanage cannot give us --------------------------------

def _rounded_mask(rgba: bytearray, inset: float, radius: float) -> bytearray:
    """Clip to a rounded square, everything outside it transparent.

    The usual signed-distance formula, with a one-pixel ramp at the edge so the
    curve is antialiased rather than a staircase.
    """
    half = SIZE / 2 - inset
    flat = half - radius
    centre = SIZE / 2
    for y in range(SIZE):
        dy = abs(y + 0.5 - centre) - flat
        for x in range(SIZE):
            dx = abs(x + 0.5 - centre) - flat
            if dx <= 0 and dy <= 0:
                d = max(dx, dy) - radius
            else:
                ox, oy = max(dx, 0.0), max(dy, 0.0)
                d = (ox * ox + oy * oy) ** 0.5 - radius
            if d >= 0.5:
                a = 0
            elif d <= -0.5:
                a = 255
            else:
                a = int(round((0.5 - d) * 255))
            i = (y * SIZE + x) * 4 + 3
            rgba[i] = min(rgba[i], a)
    return rgba


def _key_white(rgba: bytearray) -> bytearray:
    """Recover the mark's alpha from a render of it flat on white.

    A pixel is `mark·a + white·(1-a)`, and the mark is one flat colour, so `a`
    follows from any channel that differs from white — the widest one, for the
    least rounding error. Exact, unlike keying by colour similarity.
    """
    mark = tuple(int(MARK[i:i + 2], 16) for i in (1, 3, 5))
    span = max(255 - c for c in mark)
    channel = next(i for i, c in enumerate(mark) if 255 - c == span)
    for i in range(0, len(rgba), 4):
        a = (255 - rgba[i + channel]) * 255 // span
        rgba[i:i + 4] = bytes((*mark, min(255, a)))
    return rgba


# ---- checks ----------------------------------------------------------------

def px(rgba: bytearray, x: int, y: int) -> tuple[int, ...]:
    i = (y * SIZE + x) * 4
    return tuple(rgba[i:i + 4])


def expect(name: str, got, want) -> None:
    if got != want:
        sys.exit(f'{name}: got {got}, expected {want}')


def main() -> None:
    if not LOGO.exists():
        sys.exit(f'no logo at {LOGO}')
    OUT.mkdir(parents=True, exist_ok=True)
    groups = mark_groups(LOGO.read_text())
    card = tuple(int(CARD[i:i + 2], 16) for i in (1, 3, 5))

    # macOS, Linux, web: the card, clipped to Apple's rounded-square grid.
    icon = to_rgba(_render(canvas(CARD, place(groups, FILL_ICON))))
    icon = _rounded_mask(icon, MAC_INSET, MAC_RADIUS)
    expect('icon corner is transparent', px(icon, 8, 8)[3], 0)
    expect('icon centre-left is card', px(icon, SIZE // 2, 120)[:3], card)
    write_png(OUT / 'storm_icon.png', icon)

    # Android legacy and the web maskable icon: full bleed, the launcher masks.
    full = to_rgba(_render(canvas(CARD, place(groups, FILL_FULL))))
    expect('full-bleed corner is card', px(full, 2, 2)[:3], card)
    write_png(OUT / 'storm_icon_full.png', full)

    # Android adaptive foreground: the mark alone, on nothing.
    mark = _key_white(to_rgba(_render(canvas('#ffffff', place(groups, FILL_MARK)))))
    expect('mark corner is transparent', px(mark, 4, 4)[3], 0)
    if not any(mark[i] > 200 for i in range(3, len(mark), 4)):
        sys.exit('mark came out blank — nothing survived the white key')
    write_png(OUT / 'storm_mark.png', mark)

    for f in ('storm_icon.png', 'storm_icon_full.png', 'storm_mark.png'):
        print(f'  {f}  {(OUT / f).stat().st_size // 1024}K')
    print('now run: dart run icons_launcher:create')


if __name__ == '__main__':
    main()
