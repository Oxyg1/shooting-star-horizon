#!/usr/bin/env python3
"""Render trailer motion-graphics overlays (WebM VP9 + alpha, 1920x1080 @ 60 fps).

Usage:  python3 tools/trailer/titles.py [name ...]
  names: intro outro flash impact_ring card_<id> ...  (default: everything)
Outputs go to tools/trailer/assets/. Frames are rendered to a temp dir and deleted.
"""
import math, os, shutil, subprocess, sys, tempfile
import multiprocessing as mp
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter

W, H, FPS = 1920, 1080, 60
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "assets")
FD = "/usr/share/fonts/opentype/inter/"
F_DISP = FD + "InterDisplay-Bold.otf"
F_XB = FD + "Inter-ExtraBold.otf"
F_B = FD + "Inter-Bold.otf"
F_R = FD + "Inter-Regular.otf"
WHITE = (255, 255, 255)
TITLE_COL = (228, 234, 244)
COLD = (143, 208, 255)

CARDS = [
    ("the_shooting_star", "SS-01", "RAILGUN", "An orbital railgun fired from deep space", "#FF3248"),
    ("gungnir", "SS-03", "GUNGNIR", "A tungsten spear from Jupiter's orbit", "#FF5A1F"),
    ("seven_stars", "SS-04", "SEVEN STARS", "The Big Dipper falls, star by star", "#B8A8FF"),
    ("tesseract", "SS-05", "TESSERACT", "A fourth-dimensional cube lifts the land", "#7FF0FF"),
    ("event_horizon", "SS-H1", "EVENT HORIZON", "A black hole dropped on the world", "#FFA040"),
    ("supernova", "SS-H2", "SUPERNOVA", "A star swells, falls in, and explodes", "#9FD8FF"),
    ("gravitia", "SS-06", "GRAVITIA", "A ring engine around the Earth", "#B078FF"),
]
NEW_IN_PORT = {"SS-H1", "SS-H2"}

# ---------------------------------------------------------------- easing
def clamp(x, a=0.0, b=1.0): return min(max(x, a), b)
def prog(t, t0, t1): return clamp((t - t0) / (t1 - t0))
def ease_out(p): return 1 - (1 - p) ** 3
def ease_io(p): return 4 * p ** 3 if p < 0.5 else 1 - (-2 * p + 2) ** 3 / 2
def hexrgb(h): h = h.lstrip("#"); return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))

# ---------------------------------------------------------------- canvas
class Cv:
    """Premultiplied float RGBA region of the frame, origin (ox, oy)."""
    def __init__(s, region):
        s.ox, s.oy, x1, y1 = region
        s.a = np.zeros((y1 - s.oy, x1 - s.ox, 4), np.float32)

    def put(s, arr, x, y, alpha=1.0):
        x -= s.ox; y -= s.oy
        ix, iy = math.floor(x), math.floor(y)
        fx, fy = x - ix, y - iy
        if fx > 0.01 or fy > 0.01:
            h, w, _ = arr.shape
            sh = np.zeros((h + 1, w + 1, 4), np.float32)
            sh[:h, :w] += arr * ((1 - fx) * (1 - fy))
            sh[:h, 1:] += arr * (fx * (1 - fy))
            sh[1:, :w] += arr * ((1 - fx) * fy)
            sh[1:, 1:] += arr * (fx * fy)
            arr = sh
        h, w, _ = arr.shape
        H0, W0, _ = s.a.shape
        x0, y0 = max(ix, 0), max(iy, 0)
        x1, y1 = min(ix + w, W0), min(iy + h, H0)
        if x1 <= x0 or y1 <= y0: return
        src = arr[y0 - iy:y1 - iy, x0 - ix:x1 - ix] * alpha
        d = s.a[y0:y1, x0:x1]
        d[:] = src + d * (1 - src[..., 3:4])

    def transformed(s, scale, dx, dy, ax, ay, alpha):
        """Scale about absolute point (ax, ay), translate, multiply alpha."""
        h, w, _ = s.a.shape
        nw, nh = max(1, round(w * scale)), max(1, round(h * scale))
        if (nw, nh) != (w, h):
            r = np.stack([np.asarray(Image.fromarray(s.a[..., c], "F").resize((nw, nh), Image.BICUBIC))
                          for c in range(4)], -1)
        else:
            r = s.a
        out = Cv((s.ox, s.oy, s.ox + w, s.oy + h))
        lx, ly = ax - s.ox, ay - s.oy
        out.put(np.clip(r, 0, 1), s.ox + lx - lx * scale + dx, s.oy + ly - ly * scale + dy, alpha)
        return out

# ---------------------------------------------------------------- sprites
def text_mask(text, fpath, size, tracking, pad):
    S = 2
    font = ImageFont.truetype(fpath, size * S)
    xs, x = [], 0.0
    for i, ch in enumerate(text):
        xs.append(x)
        adv = font.getlength(ch)
        if i + 1 < len(text):
            adv = font.getlength(text[i:i + 2]) - font.getlength(text[i + 1])
        x += adv + tracking * S
    tw = (x - tracking * S) / S
    w = math.ceil(tw) + 2 * pad
    base = pad + int(size * 1.1)
    h = base + int(size * 0.4) + pad
    img = Image.new("L", (w * S, h * S), 0)
    d = ImageDraw.Draw(img)
    for ch, xx in zip(text, xs):
        d.text((pad * S + xx, base * S), ch, font=font, fill=255, anchor="ls")
    img = img.resize((w, h), Image.LANCZOS)
    return np.asarray(img, np.float32) / 255, tw, pad, base


def glitch(a, seed, amt=1.0):
    rng = np.random.default_rng(seed)
    h, w, _ = a.shape
    out = a.copy()
    for _ in range(int(rng.integers(4, 8))):
        y = int(rng.integers(0, max(1, h - 6)))
        sh = int(rng.integers(5, max(8, h // 4)))
        dx = int(rng.integers(-70, 71) * amt)
        out[y:y + sh] = np.roll(out[y:y + sh], dx, axis=1)
    d = max(2, int(rng.integers(7, 15) * amt))
    r, b = np.roll(out, d, axis=1), np.roll(out, -d, axis=1)
    res = out.copy()
    res[..., 0] = r[..., 0]
    res[..., 2] = b[..., 2]
    res[..., 3] = np.maximum(out[..., 3], np.maximum(r[..., 3], b[..., 3]))
    # occasional dropped slice
    y = int(rng.integers(0, max(1, h - 8)))
    res[y:y + int(rng.integers(2, 6))] *= 0.35
    return res


class Spr:
    def __init__(s, mask, bx, by, color, opacity=1.0, glow=None, glow_op=0.6,
                 glow_sigma=10, gain=2.2, tw=None):
        s.m, s.bx, s.by, s.op, s.tw = mask, bx, by, opacity, tw
        h, w = mask.shape
        col = np.array(color, np.float32) / 255
        a = mask * opacity
        rgb = mask[..., None] * col * opacity
        if glow is not None:
            g = np.asarray(Image.fromarray((mask * 255).astype(np.uint8)).filter(
                ImageFilter.GaussianBlur(glow_sigma)), np.float32) / 255
            ga = np.clip(g * gain, 0, 1) * glow_op
            gc = np.array(glow, np.float32) / 255
            rgb = rgb + (ga * (1 - a))[..., None] * gc
            a = a + ga * (1 - a)
            s.gl = ga
        else:
            s.gl = np.zeros_like(mask)
        s.arr = np.concatenate([rgb, a[..., None]], -1).astype(np.float32)
        yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
        s.U = xx + 0.4 * yy
        s.xs = np.arange(w, dtype=np.float32)

    @property
    def w(s): return s.arr.shape[1]
    @property
    def h(s): return s.arr.shape[0]

    def get(s, colmask=None, sweep=None, gl_seed=None, gl_amt=1.0):
        arr = s.arr
        if sweep is not None:
            pos = -0.4 * s.h - 140 + sweep * (s.w + 0.4 * s.h + 280)
            b = np.exp(-(((s.U - pos) / 55.0) ** 2))
            bl = b * s.gl * 0.55
            arr = arr.copy()
            arr[..., :3] += ((b * s.m * s.op * 0.6) + bl)[..., None]
            arr[..., 3] = np.minimum(1, arr[..., 3] + bl)
            arr[..., :3] = np.minimum(arr[..., :3], arr[..., 3:4])
        if colmask is not None:
            arr = arr * colmask[None, :, None]
        if gl_seed is not None:
            arr = glitch(arr, gl_seed, gl_amt)
        return arr

    def put(s, cv, x, y, alpha=1.0, **kw):
        if alpha <= 0.002: return
        cv.put(s.get(**kw), x - s.bx, y - s.by, alpha)

    def wipe_center(s, p, feather=60):
        half = p * (s.w / 2 + feather)
        return np.clip((half - np.abs(s.xs - s.w / 2)) / feather, 0, 1)

    def wipe_ltr(s, p, feather=60):
        return np.clip((p * (s.w + feather) - s.xs) / feather, 0, 1)


def text_spr(text, fpath, size, color=WHITE, tracking=0, opacity=1.0, glow=None,
             glow_op=0.6, glow_sigma=None, pad=90, gain=1.7):
    m, tw, bx, by = text_mask(text, fpath, size, tracking, pad)
    gs = glow_sigma if glow_sigma is not None else max(4, size * 0.13)
    return Spr(m, bx, by, color, opacity, glow, glow_op, gs, gain, tw)


def line_spr(length, color, opacity=1.0, thick=2, glow=None, glow_op=0.6, fade_ends=0.0, padx=50, pad=40):
    h, w = thick + 2 * pad, int(length) + 2 * padx
    m = np.zeros((h, w), np.float32)
    m[pad:pad + thick, padx:padx + int(length)] = 1
    if fade_ends > 0:
        xs = np.arange(w, dtype=np.float32) - padx
        n = length * fade_ends
        m *= np.clip(np.minimum(xs, length - xs) / n, 0, 1)[None, :] ** 0.8
    return Spr(m, padx, pad, color, opacity, glow, glow_op, 5, 2.4, length)

# ---------------------------------------------------------------- star glint
_GRID = {}
def glint_arr(e, size=700):
    """4-point star flare; e in 0..1 is the envelope."""
    if size not in _GRID:
        yy, xx = np.mgrid[0:size, 0:size].astype(np.float32)
        _GRID[size] = (xx - size / 2, yy - size / 2)
    dx, dy = _GRID[size]
    adx, ady = np.abs(dx), np.abs(dy)
    L = 330 * (0.35 + 0.65 * e)
    th = 1.3 + 5.5 * np.exp(-adx / 40)
    sh = np.exp(-adx / (L * 0.28)) * np.exp(-(dy / th) ** 2)
    thv = 1.3 + 5.5 * np.exp(-ady / 40)
    sv = 0.75 * np.exp(-ady / (L * 0.2)) * np.exp(-(dx / thv) ** 2)
    r = np.sqrt(dx * dx + dy * dy)
    core = np.exp(-(r / (16 + 22 * e)) ** 2) + 0.35 * np.exp(-(r / 70) ** 2)
    I = (sh + sv + core) * e
    a = np.clip(I, 0, 1)
    col = np.array([0.78, 0.9, 1.0], np.float32)
    rgb = np.clip(col[None, None, :] * a[..., None] + np.clip(I - 1, 0, 1)[..., None] * 0.3, 0, 1)
    rgb = np.minimum(rgb, a[..., None])
    return np.concatenate([rgb, a[..., None]], -1).astype(np.float32)

def glint_env(t, t0, t1):
    p = prog(t, t0, t1)
    if p <= 0 or p >= 1: return 0.0
    att = ease_out(min(p / 0.2, 1))
    return att * (1 - p) ** 1.5 / (0.8 ** 1.5)

# ---------------------------------------------------------------- backdrop
def backdrop(region, rect, radius, sigma, peak=0.45):
    x0, y0, x1, y1 = region
    Q = 4
    img = Image.new("L", (((x1 - x0) // Q) + 1, ((y1 - y0) // Q) + 1), 0)
    r = [(rect[0] - x0) / Q, (rect[1] - y0) / Q, (rect[2] - x0) / Q, (rect[3] - y0) / Q]
    ImageDraw.Draw(img).rounded_rectangle(r, radius / Q, fill=255)
    img = img.filter(ImageFilter.GaussianBlur(sigma / Q)).resize((x1 - x0, y1 - y0), Image.BICUBIC)
    return np.asarray(img, np.float32) / 255 * peak

# ---------------------------------------------------------------- scenes
class Scene:
    def __init__(s, T, region, bd, draw, exit_kind=None, anchor=(960, 540)):
        s.T, s.region, s.bd, s.draw, s.exit_kind, s.anchor = T, region, bd, draw, exit_kind, anchor
        s.n = round(T * FPS)

    def render(s, i):
        t = i / FPS
        cv = Cv(s.region)
        s.draw(cv, t, i)
        bf = ease_out(prog(t, 0, 0.5))
        if s.exit_kind:
            pe = clamp((t - (s.T - 0.35)) / (0.35 - 1 / FPS))
            if pe > 0:
                al = 1 - pe ** 1.3
                if s.exit_kind == "card":
                    cv = cv.transformed(1 + 0.02 * ease_out(pe), -34 * ease_out(pe), 0, *s.anchor, al)
                else:
                    cv = cv.transformed(1 + 0.04 * ease_out(pe), 0, -10 * ease_out(pe), *s.anchor, al)
                bf *= al
        a = np.clip(cv.a[..., 3], 0, 1)
        oa = a + (1 - a) * s.bd * bf
        rgb = cv.a[..., :3] / np.maximum(oa, 1e-5)[..., None]
        rgba = np.concatenate([np.clip(rgb, 0, 1), oa[..., None]], -1)
        rgba[oa < 1 / 512] = 0
        full = np.zeros((H, W, 4), np.uint8)
        x0, y0 = s.region[0], s.region[1]
        full[y0:y0 + rgba.shape[0], x0:x0 + rgba.shape[1]] = (rgba * 255 + 0.5).astype(np.uint8)
        return full


def glitch_seed(t, start, frames, base):
    k = round((t - start) * FPS)
    return (base + k) if 0 <= k < frames else None


def build_title_block(cx, size, base_y, tracking=5):
    return text_spr("THE SHOOTING STAR", F_DISP, size, TITLE_COL, tracking, glow=COLD, glow_op=0.6, pad=90)


def build_intro():
    region = (0, 120, 1920, 1000)
    title = build_title_block(960, 120, 590)
    label = text_spr("A HORIZON NATIVE MOD", F_B, 22, WHITE, 9, 0.7, pad=20)
    sub = text_spr("for Horizon  ·  Minecraft in Rust", F_R, 28, WHITE, 5, 0.85, pad=20)
    line = line_spr(title.tw, WHITE, 0.9, glow=COLD, glow_op=0.7, fade_ends=0.12)
    tick = line_spr(70, WHITE, 0.55, thick=2, fade_ends=0.0, padx=4, pad=4)
    TB, LB, LY, SB = 590, 440, 628, 684
    tx0 = 960 - title.tw / 2
    right = 960 + title.tw / 2
    def draw(cv, t, i):
        # label
        p = ease_out(prog(t, 0.2, 0.65))
        lx = 960 - label.tw / 2
        label.put(cv, lx, LB + (1 - p) * 14, p)
        tp = ease_out(prog(t, 0.3, 0.75))
        tick.put(cv, lx - 34 - 70 - (1 - tp) * 20, LB - 8, tp)
        tick.put(cv, lx + label.tw + 34 + (1 - tp) * 20, LB - 8, tp)
        # title
        pw = ease_out(prog(t, 0.5, 1.2))
        if pw > 0:
            sw = prog(t, 1.5, 2.3)
            title.put(cv, tx0, TB, 1.0, colmask=title.wipe_center(pw),
                      sweep=(ease_io(sw) if 0 < sw < 1 else None),
                      gl_seed=glitch_seed(t, 0.62, 4, 100))
        # line
        pl = ease_out(prog(t, 0.95, 1.5))
        if pl > 0:
            line.put(cv, tx0, LY, 1.0, colmask=line.wipe_center(pl, 40))
        # sub
        ps = ease_out(prog(t, 1.15, 1.65))
        sub.put(cv, 960 - sub.tw / 2, SB + (1 - ps) * 12, ps)
        # glint at the right end of the title
        e = glint_env(t, 1.15, 1.95)
        if e > 0.003:
            cv.put(glint_arr(e), right + 4 - 350, TB - 44 - 350)
    bd = backdrop(region, (180, 380, 1740, 740), 120, 85)
    return Scene(4.0, region, bd, draw, "title", (960, 550))


def build_outro():
    region = (0, 100, 1920, 1040)
    title = build_title_block(960, 90, 480, 4)
    sub = text_spr("for Horizon", F_R, 28, WHITE, 9, 0.85, pad=20)
    url = text_spr("github.com/Oxyg1/shooting-star-horizon", F_R, 34, WHITE, 3, 1.0, pad=20)
    cred = text_spr("Original mod by rimuru.dev  ·  Horizon port", F_R, 24, WHITE, 3, 0.65, pad=20)
    line = line_spr(title.tw, WHITE, 0.9, glow=COLD, glow_op=0.7, fade_ends=0.12)
    div = line_spr(90, WHITE, 0.5, fade_ends=0.3, padx=4, pad=4)
    TB, LY, SB, UB, CB = 480, 516, 574, 700, 756
    tx0 = 960 - title.tw / 2
    right = 960 + title.tw / 2
    def draw(cv, t, i):
        pw = ease_out(prog(t, 0.15, 0.85))
        if pw > 0:
            sw = prog(t, 1.0, 1.8)
            title.put(cv, tx0, TB, 1.0, colmask=title.wipe_center(pw),
                      sweep=(ease_io(sw) if 0 < sw < 1 else None),
                      gl_seed=glitch_seed(t, 0.27, 4, 200))
        pl = ease_out(prog(t, 0.6, 1.1))
        if pl > 0:
            line.put(cv, tx0, LY, 1.0, colmask=line.wipe_center(pl, 40))
        ps = ease_out(prog(t, 0.7, 1.2))
        sub.put(cv, 960 - sub.tw / 2, SB + (1 - ps) * 12, ps)
        e = glint_env(t, 0.8, 1.6)
        if e > 0.003:
            cv.put(glint_arr(e, 560), right + 4 - 280, TB - 33 - 280)
        pu = ease_out(prog(t, 1.2, 1.7))
        div.put(cv, 960 - 45, 636, pu)
        url.put(cv, 960 - url.tw / 2, UB + (1 - pu) * 14, pu)
        pc = ease_out(prog(t, 1.8, 2.3))
        cred.put(cv, 960 - cred.tw / 2, CB + (1 - pc) * 12, pc)
    bd = backdrop(region, (260, 340, 1660, 800), 140, 95)
    return Scene(6.0, region, bd, draw, None)


def build_card(cid, code, name, tag, accent):
    acc = hexrgb(accent)
    X, NB = 120, 820
    nm = text_spr(name, F_DISP, 96, (246, 248, 252), 2, glow=acc, glow_op=0.6, pad=90)
    cd = text_spr(code, F_B, 22, acc, 8, 1.0, glow=acc, glow_op=0.5, glow_sigma=4, pad=16)
    extra = text_spr("NEW IN THE PORT", F_B, 15, WHITE, 6, 0.6, pad=16) if code in NEW_IN_PORT else None
    tg = text_spr(tag, F_R, 30, WHITE, 1, 0.8, pad=16)
    bar = line_spr(24, acc, 1.0, thick=3, glow=acc, glow_op=0.6, padx=14, pad=14)  # rotated below
    # make the bar vertical
    barm = np.zeros((24 + 28, 3 + 28), np.float32); barm[14:38, 14:17] = 1
    bar = Spr(barm, 14, 14, acc, 1.0, acc, 0.6, 4, 2.4)
    line = line_spr(nm.tw, acc, 1.0, thick=2, glow=acc, glow_op=0.7)
    right = int(X + max(nm.tw, tg.tw) + 260)
    region = (0, 380, min(1920, right + 420), 1080)
    bd = backdrop(region, (-300, 640, right, 960), 120, 80)
    CB = 712
    def draw(cv, t, i):
        p = ease_out(prog(t, 0.0, 0.35))
        bar.put(cv, X, 692 - 0 + 0, p)  # bar top
        cd.put(cv, X + 24 - (1 - p) * 30, CB, p)
        if extra:
            extra.put(cv, X + 24 + cd.tw + 26 - (1 - p) * 30, CB - 1, p * 0.999)
        pn = ease_out(prog(t, 0.05, 0.45))
        if pn > 0:
            sw = prog(t, 0.6, 1.4)
            nm.put(cv, X - (1 - pn) * 44, NB, 1.0, colmask=nm.wipe_ltr(pn, 90),
                   sweep=(ease_io(sw) if 0 < sw < 1 else None),
                   gl_seed=glitch_seed(t, 0.12, 3, 300 + len(name)), gl_amt=0.8)
        pl = ease_out(prog(t, 0.12, 0.45))
        if pl > 0:
            line.put(cv, X, 846, 1.0, colmask=line.wipe_ltr(pl, 30))
        pt = ease_out(prog(t, 0.2, 0.45))
        tg.put(cv, X, 902 + (1 - pt) * 12, pt)
    return Scene(2.6, region, bd, draw, "card", (X, NB))

# ---------------------------------------------------------------- flash / ring
class Flash:
    n = round(0.25 * FPS)
    def render(s, i):
        p = i / (s.n - 1)
        a = 0.9 * (ease_out(p / 0.3) if p < 0.3 else (1 - (p - 0.3) / 0.7) ** 2)
        if i == 0 or i == s.n - 1: a = 0.0
        out = np.zeros((H, W, 4), np.uint8)
        out[...] = (255, 255, 255, int(round(a * 255)))
        return out


class Ring:
    n = round(0.6 * FPS)
    def __init__(s):
        yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
        s.d = np.sqrt((xx + 0.5 - W / 2) ** 2 + (yy + 0.5 - H / 2) ** 2)
    def render(s, i):
        p = i / (s.n - 1)
        r = 24 + 860 * ease_out(p)
        hw = 0.5 + 1.3 * (1 - p)
        fade = min(1, (i + 1) / 3) * (1 - p) ** 1.5
        a = np.zeros((H, W), np.float32)
        for rr, k in ((r, 1.0), (r * 0.72, 0.3)):
            d = np.abs(s.d - rr)
            core = np.clip(hw + 0.5 - d, 0, 1)
            glow = 0.5 * np.exp(-(d / (10 + 16 * p)) ** 2) + 0.2 * np.exp(-(d / (40 + 30 * p)) ** 2)
            a = np.maximum(a, np.clip(core + glow, 0, 1) * k)
        a *= fade
        out = np.zeros((H, W, 4), np.uint8)
        out[..., :3] = 255
        out[..., 3] = (a * 255 + 0.5).astype(np.uint8)
        return out

# ---------------------------------------------------------------- driver
_SC = None
def _work(args):
    i, d = args
    Image.fromarray(_SC.render(i), "RGBA").save(os.path.join(d, f"frame_{i:04d}.png"), compress_level=1)
    return i

def render_webm(name, scene):
    global _SC
    _SC = scene
    d = tempfile.mkdtemp(prefix=f"ttl_{name}_")
    try:
        with mp.get_context("fork").Pool(os.cpu_count() or 2) as pool:
            for _ in pool.imap_unordered(_work, [(i, d) for i in range(scene.n)], chunksize=4):
                pass
        out = os.path.join(OUT, name + ".webm")
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(FPS), "-i",
                        os.path.join(d, "frame_%04d.png"), "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p",
                        "-b:v", "0", "-crf", "30", "-auto-alt-ref", "0", "-deadline", "good",
                        "-cpu-used", "3", "-row-mt", "1", "-threads", str(os.cpu_count() or 2), out], check=True)
        print(f"{name}.webm  {os.path.getsize(out) / 1e6:.2f} MB  ({scene.n} frames)", flush=True)
    finally:
        shutil.rmtree(d, ignore_errors=True)

def main():
    os.makedirs(OUT, exist_ok=True)
    jobs = {"intro": build_intro, "outro": build_outro, "flash": Flash, "impact_ring": Ring}
    for c in CARDS:
        jobs["card_" + c[0]] = (lambda c=c: build_card(*c))
    want = sys.argv[1:] or list(jobs)
    for n in want:
        render_webm(n, jobs[n]())

if __name__ == "__main__":
    main()
