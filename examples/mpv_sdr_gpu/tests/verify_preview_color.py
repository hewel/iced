#!/usr/bin/env python3
"""Compare actual worker pixels with independent ST2084/BT.2100 and sRGB math.

No FFmpeg color filter generates fixtures or expected pixels. FFV1 only encodes
our explicitly quantized RGB planes losslessly. Reports/media stay outside Git.
"""
import argparse
import array
import json
import math
import os
from pathlib import Path
import subprocess
import sys


def inverse(m):
    a, b, c = m[0]
    d, e, f = m[1]
    g, h, i = m[2]
    det = a * (e*i-f*h) - b * (d*i-f*g) + c * (d*h-e*g)
    return [[v / det for v in row] for row in
            [[e*i-f*h, c*h-b*i, b*f-c*e],
             [f*g-d*i, a*i-c*g, c*d-a*f],
             [d*h-e*g, b*g-a*h, a*e-b*d]]]


def mv(m, v):
    return [sum(a*b for a, b in zip(row, v)) for row in m]


def rgb_xyz(primaries):
    columns = [(x/y, 1, (1-x-y)/y) for x, y in primaries]
    m = [list(row) for row in zip(*columns)]
    white = (.3127/.3290, 1, (1-.3127-.3290)/.3290)
    scale = mv(inverse(m), white)
    return [[v*scale[j] for j, v in enumerate(row)] for row in m]


RGB2020 = rgb_xyz([(.708, .292), (.170, .797), (.131, .046)])
XYZ709 = inverse(rgb_xyz([(.640, .330), (.300, .600), (.150, .060)]))


def pq_encode(nits):
    m1, m2 = 2610/16384, 2523/32
    c1, c2, c3 = 3424/4096, 2413/128, 2392/128
    p = (nits/10000)**m1
    return ((c1+c2*p)/(1+c3*p))**m2


def pq_decode(code):
    m1, m2 = 2610/16384, 2523/32
    c1, c2, c3 = 3424/4096, 2413/128, 2392/128
    p = code**(1/m2)
    return 10000*(max(p-c1, 0)/(c2-c3*p))**(1/m1)


def hlg_encode(scene):
    a = .17883277
    b = 1-4*a
    c = .5-a*math.log(4*a)
    return math.sqrt(3*scene) if scene <= 1/12 else a*math.log(12*scene-b)+c


def hlg_decode(code):
    a = .17883277
    b = 1-4*a
    c = .5-a*math.log(4*a)
    return code*code/3 if code <= .5 else (math.exp((code-c)/a)+b)/12


def hable(x):
    return (x*(.15*x+.05)+.004)/(x*(.15*x+.5)+.06)-.02/.30


def srgb(x):
    x = max(0, min(1, x))
    return 12.92*x if x <= .0031308 else 1.055*x**(1/2.4)-.055


def expected(codes, transfer):
    if transfer == "pq":
        linear = [pq_decode(v)/100 for v in codes]
    else:
        scene = [hlg_decode(v) for v in codes]
        luminance = sum(a*b for a, b in zip(scene, [.2627, .6780, .0593]))
        # BT.2100 reference 1000-nit display, system gamma 1.2, npl=100.
        linear = [10*v*luminance**.2 for v in scene]
    linear = mv(XYZ709, mv(RGB2020, linear))
    maximum = max(max(linear), 1e-6)
    scale = hable(maximum)/hable(10)/maximum
    return [round(255*srgb(v*scale)) for v in linear]


def run(args):
    subprocess.run(["timeout", "--kill-after=5s", "45s", *map(str, args)], check=True)


def compare_real(media, kind, report, library):
    """Integration reference, separate from the independent analytic fixtures."""
    output = report/kind
    run(["cargo", "run", "--quiet", "-p", "mpv_sdr_gpu", "--example", "preview_smoke", "--",
         media, "--libmpv", library, "--capture-at", "300", "--report", output])
    metadata = (output/"300.txt").read_text()
    required = "DV8 HDR10 base" if kind == "dv8" else "PQ HDR"
    if required not in metadata:
        raise SystemExit(f"FAIL unexpected worker color mode: {metadata}")
    actual_time = metadata.split("actual=", 1)[1].split()[0]
    reference = output/"reference.rgb"
    # This checks the mpv bridge/BL handling against direct FFmpeg decoding.
    # The analytic fixture mode, not this shared filter reference, proves math.
    graph = ("zscale=w=320:h=180:t=linear:npl=100:agamma=0,format=gbrpf32le,"
             "zscale=p=bt709,tonemap=tonemap=hable:desat=0,"
             "zscale=t=iec61966-2-1:p=bt709:r=full:dither=none,format=gbrp,format=rgb24")
    run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-ss", actual_time,
         "-i", media, "-map", "0:v:0", "-frames:v", "1", "-vf", graph,
         "-f", "rawvideo", reference])
    rgba = (output/"300.rgba").read_bytes()
    rgb = bytes(v for i, v in enumerate(rgba) if i % 4 != 3)
    expected_pixels = reference.read_bytes()
    if len(rgb) != len(expected_pixels):
        raise SystemExit("FAIL real-media reference dimensions")
    differences = [abs(a-b) for a, b in zip(rgb, expected_pixels)]
    result = dict(media=str(media), mode=kind, max_error=max(differences),
                  mean_error=sum(differences)/len(differences))
    (output/"comparison.json").write_text(json.dumps(result, indent=2)+"\n")
    print(result)
    if result["max_error"] > 3:
        raise SystemExit("FAIL worker differs from direct FFmpeg HDR/base-layer conversion")
    print(f"PASS {kind} real-media color integration; worker joined and process exited")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--real-pq", type=Path, help="Compare a 16:9 PQ sample at 300 seconds")
    parser.add_argument("--real-dv8", type=Path, help="Compare a 16:9 DV8 HDR10-base sample at 300 seconds")
    options = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    os.chdir(root)
    report = options.report.resolve()
    report.mkdir(parents=True, exist_ok=True)
    library = os.environ.get("LIBMPV", "/home/hewel/Codes/mpv/build/libmpv.so")
    pl = os.environ.get("PL_LIBDIR", str(Path.home()/".cache/mpv-preview-deps/libplacebo-7.387.0/install/lib"))
    os.environ["LD_LIBRARY_PATH"] = pl + (":"+os.environ["LD_LIBRARY_PATH"] if os.environ.get("LD_LIBRARY_PATH") else "")
    subprocess.run(["cargo", "build", "-p", "mpv_sdr_gpu", "--example", "preview_smoke"], check=True)
    if options.real_pq or options.real_dv8:
        for kind, media in [("pq", options.real_pq), ("dv8", options.real_dv8)]:
            if media:
                compare_real(media, kind, report, library)
        return
    results = []
    for transfer in ["pq", "hlg"]:
        if transfer == "pq":
            gray = [0, .1, 1, 10, 100, 203, 400, 1000]
            colors = [(100, 0, 0), (0, 100, 0), (0, 0, 100), (100, 30, 5),
                      (20, 100, 30), (20, 30, 100), (100, 100, 0), (200, 100, 50)]
            encode = pq_encode
            trc = "smpte2084"
        else:
            gray = [0, .001, .01, .1, .18, .25, .5, 1]
            colors = [(.1, 0, 0), (0, .1, 0), (0, 0, .1), (.1, .03, .005),
                      (.02, .1, .03), (.02, .03, .1), (.1, .1, 0), (.2, .1, .05)]
            encode = hlg_encode
            trc = "arib-std-b67"
        patches = [(v, v, v) for v in gray] + colors
        quantized = [[round(encode(v)*65535) for v in rgb] for rgb in patches]
        planes = array.array("H", (quantized[(y//90)*8+x//40][c]
                                   for c in [1, 2, 0] for y in range(180) for x in range(320)))
        if sys.byteorder != "little":
            planes.byteswap()
        raw = report/f"{transfer}.gbrp16le"
        raw.write_bytes(planes.tobytes()*12)
        media = report/f"{transfer}.mkv"
        run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo",
             "-pixel_format", "gbrp16le", "-video_size", "320x180", "-framerate", "1",
             "-color_trc", trc, "-color_primaries", "bt2020", "-colorspace", "0", "-color_range", "pc",
             "-i", raw, "-c:v", "ffv1", "-level", "3", "-pix_fmt", "gbrp16le", media])
        output = report/transfer
        run(["cargo", "run", "--quiet", "-p", "mpv_sdr_gpu", "--example", "preview_smoke", "--",
             media, "--libmpv", library, "--capture-at", "0.5", "--report", output])
        pixels = (output/"0.5.rgba").read_bytes()
        errors = []
        for i, values in enumerate(quantized):
            x, y = 20+40*(i%8), 45+90*(i//8)
            offset = (y*320+x)*4
            actual = list(pixels[offset:offset+3])
            reference = expected([v/65535 for v in values], transfer)
            error = max(abs(a-b) for a, b in zip(actual, reference))
            errors.append(error)
            results.append(dict(transfer=transfer, patch=i, input=patches[i], expected=reference, actual=actual, max_error=error))
            print(f"{transfer} patch={i:02} expected={reference} actual={actual} max_error={error}")
        (report/"color-results.json").write_text(json.dumps(results, indent=2)+"\n")
        if max(errors) > 3:
            raise SystemExit(f"FAIL {transfer}: color error {max(errors)} exceeds 3 eight-bit codes")
    print("PASS PQ/HLG gray, gamut conversion, linear tone mapping and sRGB worker pixels (<=3 codes)")


if __name__ == "__main__":
    main()
