"""The port's own sound design for SS-H1 Event Horizon and SS-H2 Supernova.

Everything here is synthesised from nothing (oscillators, noise, filters,
convolution reverb), so the port ships no one else's audio. Run it to make
the .ogg files and `sounds.json` of the `ss_port` namespace:

    python3 tools/sounds/synth.py native/data/content/assets/ss_port

Needs numpy, scipy and ffmpeg (with libvorbis).
"""
import json
import os
import subprocess
import sys
import tempfile

import numpy as np
from scipy import signal

SR = 44100
RNG = np.random.default_rng(20261107)


# ---- building blocks -----------------------------------------------------------------------------------------------

def t_axis(seconds):
    return np.arange(int(seconds * SR)) / SR


def env_adsr(n, a, d, s, r, curve=3.0):
    """Attack, decay to sustain, release (seconds), exponential-ish."""
    out = np.ones(n) * s
    na, nd, nr = int(a * SR), int(d * SR), int(r * SR)
    if na:
        out[:na] = np.linspace(0, 1, na) ** 0.6
    if nd:
        out[na:na + nd] = s + (1 - s) * np.exp(-curve * np.linspace(0, 1, nd))
    if nr:
        out[max(0, n - nr):] *= np.linspace(1, 0, min(nr, n)) ** 2
    return out


def exp_decay(n, seconds):
    return np.exp(-np.arange(n) / (seconds * SR))


def noise(n, color="white"):
    w = RNG.standard_normal(n)
    if color == "white":
        return w
    spec = np.fft.rfft(w)
    f = np.fft.rfftfreq(n, 1 / SR)
    f[0] = f[1]
    if color == "pink":
        spec /= np.sqrt(f)
    elif color == "brown":
        spec /= f
    out = np.fft.irfft(spec, n)
    return out / (np.std(out) + 1e-12)


def sos(kind, freq, order=2, q=None):
    if kind == "band":
        lo, hi = freq
        return signal.butter(order, [lo, hi], btype="bandpass", fs=SR, output="sos")
    return signal.butter(order, freq, btype=kind, fs=SR, output="sos")


def filt(x, kind, freq, order=2):
    return signal.sosfilt(sos(kind, freq, order), x)


def sweep_filter(x, kind, f0, f1, order=2, block=512, curve="exp"):
    """A filter whose cutoff glides from f0 to f1 over the sound."""
    n = len(x)
    out = np.zeros(n)
    zi = None
    steps = max(1, n // block)
    for i in range(steps + 1):
        a, b = i * block, min(n, (i + 1) * block)
        if a >= n:
            break
        k = a / max(1, n - 1)
        fc = f0 * (f1 / f0) ** k if curve == "exp" else f0 + (f1 - f0) * k
        fc = min(max(fc, 15.0), SR * 0.45)
        s = sos(kind, fc, order)
        if zi is None:
            zi = signal.sosfilt_zi(s) * 0.0
        out[a:b], zi = signal.sosfilt(s, x[a:b], zi=zi)
    return out


def osc(freq, n, kind="sine", phase=0.0):
    """Oscillator with a frequency curve (array or number); band-limited saw by additive partials."""
    f = np.broadcast_to(np.asarray(freq, dtype=float), (n,))
    ph = 2 * np.pi * np.cumsum(f) / SR + phase
    if kind == "sine":
        return np.sin(ph)
    if kind == "saw":
        out = np.zeros(n)
        top = float(np.max(f))
        for k in range(1, int(min(60, (SR * 0.45) / max(top, 1)))):
            out += np.sin(ph * k) / k
        return out * 0.6
    if kind == "tri":
        return 2 / np.pi * np.arcsin(np.sin(ph))
    raise ValueError(kind)


def glide(f0, f1, n, curve=1.0):
    k = np.linspace(0, 1, n) ** curve
    return f0 * (f1 / f0) ** k


def soft(x, drive=1.0):
    return np.tanh(x * drive) / np.tanh(drive)


def impulse_response(seconds, damp_from=9000, damp_to=1200, stereo=True, predelay=0.012, early=True):
    """A hall: decaying noise, darker as it fades, slightly different in each ear."""
    n = int(seconds * SR)
    chans = []
    for c in range(2 if stereo else 1):
        tail = noise(n) * np.exp(-np.arange(n) / (seconds * SR / 6.9))
        tail = sweep_filter(tail, "low", damp_from, damp_to, order=1, block=1024)
        pre = np.zeros(int(predelay * SR))
        ir = np.concatenate([pre, tail])
        if early:
            for k in range(8):
                at = int((0.005 + RNG.random() * 0.06) * SR)
                ir[at] += (RNG.random() - 0.5) * 0.8
        chans.append(ir / np.sqrt(np.sum(ir ** 2)))
    return np.stack(chans)


def reverb(x, ir, wet=0.35):
    """x mono or stereo -> stereo, convolved with the hall."""
    if x.ndim == 1:
        x = np.stack([x, x])
    out = []
    for c in range(2):
        w = signal.fftconvolve(x[c], ir[c % len(ir)])[: len(x[c]) + len(ir[0]) - 1]
        d = np.zeros_like(w)
        d[: x.shape[1]] = x[c]
        out.append(d * (1 - wet) + w * wet * 3.0)
    return np.stack(out)


def widen(x, amount=0.6, delay_ms=11.0):
    """Mono to stereo: a short Haas offset and decorrelated tops."""
    if x.ndim == 2:
        return x
    d = int(delay_ms / 1000 * SR)
    left = x
    right = np.concatenate([np.zeros(d), x[:-d]]) if d else x
    hi_l = filt(left, "high", 1500)
    hi_r = filt(right, "high", 1500)
    mid = filt(x, "low", 1500)
    return np.stack([mid + hi_l * (1 + amount) / 2 + hi_r * (1 - amount) / 2,
                     mid + hi_r * (1 + amount) / 2 + hi_l * (1 - amount) / 2])


def pan(x, p):
    """p -1 left .. 1 right (array or number)."""
    p = np.broadcast_to(np.asarray(p, dtype=float), x.shape[-1:])
    a = (p + 1) * np.pi / 4
    return np.stack([x * np.cos(a), x * np.sin(a)])


def place(total, piece, at):
    """Add piece (mono or stereo) into total (stereo) at `at` seconds."""
    if piece.ndim == 1:
        piece = np.stack([piece, piece])
    a = int(at * SR)
    b = min(total.shape[1], a + piece.shape[1])
    total[:, a:b] += piece[:, : b - a]
    return total


def cut_at(x, seconds, fall=0.02):
    """Ends the sound at `seconds`, tail and all: for the ones the next sound must follow in silence."""
    n = int(seconds * SR)
    x = x[..., :n].copy()
    f = int(fall * SR)
    x[..., n - f:] *= np.linspace(1, 0, f)
    return x


def silence(seconds, stereo=True):
    return np.zeros((2, int(seconds * SR))) if stereo else np.zeros(int(seconds * SR))


def fade(x, fin=0.005, fout=0.05):
    n = x.shape[-1]
    e = np.ones(n)
    a, b = int(fin * SR), int(fout * SR)
    if a:
        e[:a] = np.linspace(0, 1, a)
    if b:
        e[n - b:] *= np.linspace(1, 0, b)
    return x * e


# ---- elements ------------------------------------------------------------------------------------------------------

def sub_drop(seconds, f0, f1, decay, drive=1.6):
    n = int(seconds * SR)
    body = osc(glide(f0, f1, n, 0.5), n) * exp_decay(n, decay)
    body += 0.35 * osc(glide(f0 * 2, f1 * 2, n, 0.5), n) * exp_decay(n, decay * 0.5)
    return soft(body * 1.2, drive)


def crack(seconds=0.35, lo=900, hi=7000, decay=0.06, click=True):
    n = int(seconds * SR)
    x = filt(noise(n), "band", (lo, hi), 2) * exp_decay(n, decay)
    if click:
        c = np.zeros(n)
        c[:40] = np.hanning(80)[:40][::-1] * 3
        x += filt(c, "high", 200)
    return soft(x * 2.5, 2.0)


def rumble(seconds, cutoff=180, decay=2.5, wobble=0.35):
    n = int(seconds * SR)
    x = filt(noise(n, "brown"), "low", cutoff, 2)
    lfo = 1 + wobble * filt(noise(n), "low", 3, 1) * 8
    return x * exp_decay(n, decay) * np.clip(lfo, 0.2, 2.0)


def grains(seconds, rate, lo, hi, length=(0.01, 0.06), decay=None, seed_shift=0):
    """Many short filtered noise bursts: rock grinding, crackle, debris."""
    n = int(seconds * SR)
    out = np.zeros(n)
    count = int(rate * seconds)
    for _ in range(count):
        at = int(RNG.random() * n)
        ln = int((length[0] + RNG.random() * (length[1] - length[0])) * SR)
        if at + ln >= n or ln < 16:
            continue
        g = noise(ln) * np.exp(-np.linspace(0, 6, ln))
        f = lo * (hi / lo) ** RNG.random()
        g = filt(g, "band", (f * 0.7, min(f * 1.4, SR * 0.45)), 1)
        out[at:at + ln] += g * (0.3 + RNG.random())
    if decay:
        out *= exp_decay(n, decay)
    return out


def drone_cluster(seconds, roots, detune=0.25, cutoff=(200, 900), kind="saw"):
    n = int(seconds * SR)
    out = np.zeros(n)
    for r in roots:
        for d in (-detune, 0, detune):
            f = r * 2 ** (d / 12)
            drift = 1 + 0.002 * filt(noise(n), "low", 0.5, 1) * 30
            out += osc(f * drift, n, kind, phase=RNG.random() * 6.28)
    out /= len(roots) * 3
    return sweep_filter(out, "low", cutoff[0], cutoff[1], order=2)


def shepard_rise(seconds, base=55.0, octaves=6, speed=0.6, rising=True):
    """The endless rise: octave-spaced tones gliding up under a fixed bell envelope."""
    n = int(seconds * SR)
    t = np.arange(n) / SR
    out = np.zeros(n)
    for k in range(octaves):
        pos = (k + (speed * t if rising else -speed * t)) % octaves
        f = base * 2 ** pos
        amp = np.exp(-((pos - octaves / 2) ** 2) / (2 * (octaves / 5) ** 2))
        out += osc(f, n) * amp
    return out / octaves * 2.0


def shimmer(seconds, freqs, decay=3.0, spread=0.004):
    n = int(seconds * SR)
    out = np.zeros(n)
    for f in freqs:
        for d in (-spread, spread):
            vib = 1 + 0.003 * np.sin(2 * np.pi * (4 + RNG.random() * 3) * np.arange(n) / SR)
            out += osc(f * (1 + d) * vib, n, phase=RNG.random() * 6.28) * (0.6 + 0.4 * RNG.random())
    return out / (len(freqs) * 2) * exp_decay(n, decay)


def choir(seconds, notes, vowel=(730, 1090, 2440)):
    n = int(seconds * SR)
    src = np.zeros(n)
    for f in notes:
        for v in range(4):
            vib = 1 + 0.004 * np.sin(2 * np.pi * (5 + v * 0.3) * np.arange(n) / SR + RNG.random() * 6)
            src += osc(f * 2 ** ((RNG.random() - 0.5) * 0.1 / 12) * vib, n, "saw", phase=RNG.random() * 6.28)
    src /= len(notes) * 4
    out = np.zeros(n)
    for i, fc in enumerate(vowel):
        out += filt(src, "band", (fc * 0.85, fc * 1.15), 2) * (1.0, 0.6, 0.35)[i]
    out += filt(src, "low", 400) * 0.3
    return out + filt(noise(n), "band", (2500, 7000), 1) * 0.015


# ---- loudness and files --------------------------------------------------------------------------------------------

def normalise(x, peak_db=-1.0, lufs=None):
    """Peak-limit to `peak_db`; with `lufs`, aim the loudness first (RMS approximation, k-weighted-ish)."""
    if lufs is not None:
        k = filt(x if x.ndim == 1 else x.mean(axis=0), "high", 100)
        rms = np.sqrt(np.mean(k ** 2) + 1e-12)
        x = x * (10 ** ((lufs + 0.7) / 20) / rms)
    peak = np.max(np.abs(x)) + 1e-12
    limit = 10 ** (peak_db / 20)
    if peak > limit:
        x = soft(x / peak * 1.25, 1.2) * limit if lufs is not None else x / peak * limit
    return x


def measure(x):
    """Integrated loudness (LUFS) as ffmpeg's EBU R128 meter reads it."""
    data = x.T if x.ndim == 2 else x[:, None]
    with tempfile.NamedTemporaryFile(suffix=".raw", delete=False) as f:
        f.write((np.clip(data, -4, 4) * 8000).astype("<i2").tobytes())
        raw = f.name
    out = subprocess.run(["ffmpeg", "-hide_banner", "-f", "s16le", "-ar", str(SR), "-ac", str(data.shape[1]), "-i", raw,
                          "-af", "ebur128", "-f", "null", "-"], capture_output=True, text=True).stderr
    os.unlink(raw)
    value = [line for line in out.splitlines() if line.strip().startswith("I:")][-1]
    return float(value.split()[1]) - 20 * np.log10(8000 / 32767)


def limit(x, ceiling_db=-2.0):
    """A soft limiter: under the ceiling, peaks rounded off instead of clipped."""
    c = 10 ** (ceiling_db / 20)
    knee = c * 0.7
    a = np.abs(x)
    over = a > knee
    y = x.copy()
    y[over] = np.sign(x[over]) * (knee + (c - knee) * np.tanh((a[over] - knee) / (c - knee)))
    return y


def write(folder, name, x, lufs=-13.0):
    x = x / (np.max(np.abs(x)) + 1e-12) * 0.5
    for _ in range(3):
        x = limit(x * 10 ** ((lufs - measure(x)) / 20))
    x = fade(x, 0.002, 0.03)
    os.makedirs(os.path.join(folder, "sounds"), exist_ok=True)
    data = x.T if x.ndim == 2 else x[:, None]
    pcm = (np.clip(data, -1, 1) * 32767).astype("<i2")
    with tempfile.NamedTemporaryFile(suffix=".raw", delete=False) as f:
        f.write(pcm.tobytes())
        raw = f.name
    out = os.path.join(folder, "sounds", name + ".ogg")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "s16le", "-ar", str(SR), "-ac", str(data.shape[1]),
                    "-i", raw, "-c:a", "libvorbis", "-q:a", "6", out], check=True)
    os.unlink(raw)
    return out


# ---- SS-H1 Event Horizon -------------------------------------------------------------------------------------------

HALL = None


def hall():
    global HALL
    if HALL is None:
        HALL = impulse_response(4.5, 7000, 900)
    return HALL


def eh_arm():
    """The remote wakes: a glassy rise, a lock, a low thump under it."""
    s = 2.6
    n = int(s * SR)
    rise = osc(glide(180, 1800, n, 0.7), n, "tri") * env_adsr(n, 0.9, 0.4, 0.0, 0.1)
    rise = filt(rise, "high", 150) * 0.5
    lock = np.zeros(n)
    for i, at in enumerate((0.95, 1.08, 1.21)):
        b = shimmer(0.5, [1760 * 2 ** (i * 4 / 12), 2637], decay=0.12)
        lock[int(at * SR):int(at * SR) + len(b)] += b * 1.6
    thump = np.zeros(n)
    th = sub_drop(1.4, 90, 38, 0.35)
    thump[int(0.95 * SR):int(0.95 * SR) + len(th)] += th[: n - int(0.95 * SR)]
    mix = rise + lock + thump * 0.9
    return reverb(widen(mix), hall(), 0.3)


def eh_crush():
    """The satellite crushed to a point: metal groaning, creaks, a final pop into a swell of sub."""
    s = 4.0
    n = int(s * SR)
    # Groaning metal: resonant combs over noise, their pitch sinking.
    groan = np.zeros(n)
    for f0 in (97, 143, 211, 307):
        src = noise(n) * 0.6
        res = src
        tone = osc(glide(f0, f0 * 0.42, n, 1.4), n, "saw") * 0.4
        g = filt(tone + 0.15 * res, "band", (60, 1400), 2)
        groan += g * (0.5 + 0.5 * np.abs(np.sin(2 * np.pi * (0.7 + RNG.random()) * np.arange(n) / SR)))
    groan *= env_adsr(n, 0.4, 0.5, 0.8, 0.3)
    creak = grains(s, 22, 600, 4200, (0.02, 0.09)) * 0.9
    # Squeezed: a whine climbing as it shrinks.
    whine = osc(glide(600, 5200, n, 2.5), n) * np.linspace(0, 1, n) ** 3 * 0.25
    mix = soft(groan * 1.4, 1.6) * 0.7 + creak + whine
    pop_at = 3.15
    mix[int(pop_at * SR):] *= np.exp(-np.arange(n - int(pop_at * SR)) / (0.03 * SR))
    pop = crack(0.4, 1500, 9000, 0.02)
    swell = sub_drop(0.9, 30, 50, 0.6, 1.2)
    out = widen(mix)
    place(out, pop * 1.2, pop_at)
    place(out, swell * 0.9, pop_at)
    return reverb(out, hall(), 0.32)


def eh_fall():
    """Three seconds of falling: a roar dropping in pitch, a falling tone, the air torn louder and louder."""
    s = 3.05
    n = int(s * SR)
    k = np.linspace(0, 1, n)
    roar = sweep_filter(noise(n, "pink"), "low", 5000, 300, 2)
    roar = filt(roar, "high", 60) * (0.2 + 0.8 * k ** 1.8)
    tone = osc(glide(140, 38, n, 0.8), n) * (0.3 + 0.7 * k ** 2)
    tone += 0.4 * osc(glide(280, 76, n, 0.8), n, "tri") * k ** 2
    whistle = osc(glide(2400, 380, n, 0.6), n) * np.sin(np.pi * k) ** 2 * 0.12
    # Doppler-like shudder as it nears.
    trem = 1 + 0.35 * np.sin(2 * np.pi * np.cumsum(glide(4, 22, n)) / SR) * k
    mix = (roar * 0.9 + soft(tone * 1.3, 1.5) * 0.8 + whistle) * trem
    mix *= np.minimum(1, (1 - k) * 60)  # ends hard on the landing
    return cut_at(reverb(widen(mix, 0.8), hall(), 0.22), s)


def eh_land():
    """It lands: a sub drop, the crack of the ground, the land settling."""
    s = 8.0
    out = silence(s)
    place(out, sub_drop(3.0, 72, 24, 0.9, 2.2) * 1.3, 0.0)
    place(out, widen(crack(0.6, 600, 6000, 0.09)) * 1.1, 0.0)
    place(out, widen(rumble(7.5, 220, 2.2)) * 0.9, 0.05)
    place(out, widen(grains(6.0, 60, 300, 3000, (0.02, 0.12), decay=1.6)) * 0.5, 0.15)
    return reverb(out, hall(), 0.3)


def eh_feed():
    """Nine seconds of feeding: the disk's hum rising, the wind sucked in, a beat quickening, all swelling."""
    s = 9.2
    n = int(s * SR)
    k = np.linspace(0, 1, n)
    bend = 2 ** (5 * k ** 1.5 / 12)  # five semitones up over the feed
    hum = np.zeros(n)
    for r in (36.0, 54.0, 72.0, 108.0):
        for d in (-0.18, 0.0, 0.21):
            hum += osc(r * 2 ** (d / 12) * bend, n, "saw", phase=RNG.random() * 6)
    hum /= 12
    hum = sweep_filter(hum, "low", 160, 1400, 2)
    # The gravity's wobble: a beat that quickens from slow to a flutter.
    rate = glide(0.6, 7.5, n, 1.6)
    wub = 0.55 + 0.45 * np.sin(2 * np.pi * np.cumsum(rate) / SR)
    hum *= wub
    wind = sweep_filter(noise(n, "pink"), "low", 400, 4500, 2)
    wind = filt(wind, "high", 120) * (0.15 + 0.6 * k ** 1.5)
    # Air spiralling in, panned round.
    spiral = pan(wind, np.sin(2 * np.pi * np.cumsum(glide(0.2, 1.6, n)) / SR) * 0.8)
    beat = np.zeros(n)
    t = 0.4
    while t < s - 0.3:
        b = sub_drop(0.5, 62, 34, 0.12, 1.8)
        a = int(t * SR)
        beat[a:a + len(b)] += b[: n - a] * (0.5 + 0.5 * t / s)
        t += 1.1 - 0.75 * (t / s) ** 1.2
    choir_up = choir(s, [110 * bend[0], 165, 220]) * k ** 3 * 0.5
    mono = soft(hum * 1.5, 1.4) * 0.9 + beat * 0.9 + choir_up
    out = widen(mono, 0.5) + spiral * 0.8
    out *= env_adsr(n, 1.2, 0.1, 1.0, 0.0)
    # It stops dead when the hole starts to shrink (the collapse takes over).
    return cut_at(reverb(out, hall(), 0.25), 9.0, 0.03)


def eh_tear():
    """A piece of the land torn off and pulled away (heard where it happens)."""
    s = 1.6
    n = int(s * SR)
    g = grains(s, 140, 180, 2600, (0.01, 0.07)) * env_adsr(n, 0.02, 0.4, 0.4, 0.8)
    body = rumble(s, 300, 0.5) * 1.5
    whoosh = sweep_filter(noise(n, "pink"), "low", 300, 2500, 2) * np.linspace(0, 1, n) ** 2 * 0.5
    return soft((g + body + whoosh) * 1.4, 1.5)


def eh_collapse():
    """It shrinks to nothing: everything drawn in, a whine racing up, then nothing at all."""
    s = 0.82
    n = int(s * SR)
    k = np.linspace(0, 1, n)
    hit = sub_drop(1.5, 60, 40, 0.5) + crack(1.5, 400, 4000, 0.3) * 0.5
    rev = reverb(hit, impulse_response(1.6, 6000, 1500), 0.8)[0][: int(1.6 * SR)][::-1][-n:]
    whine = osc(glide(380, 4200, n, 2.2), n) * k ** 2 * 0.4
    flutter = 1 + 0.6 * np.sin(2 * np.pi * np.cumsum(glide(8, 60, n)) / SR)
    mix = rev * 1.4 + whine * flutter + sweep_filter(noise(n, "pink"), "low", 300, 9000, 2) * k ** 3 * 0.5
    mix *= np.minimum(1, (1 - k) * 400)  # the cut
    return widen(mix, 0.7)


def eh_flash():
    """It evaporates: a white crack, the deepest drop, a glassy ringing and the world shaking for seconds after."""
    s = 12.0
    out = silence(s)
    place(out, widen(crack(0.5, 2000, 12000, 0.03)) * 1.4, 0.0)
    place(out, sub_drop(4.5, 95, 22, 1.4, 2.5) * 1.5, 0.0)
    mid = filt(noise(int(3 * SR), "pink"), "band", (150, 2500), 2) * exp_decay(int(3 * SR), 0.5)
    place(out, widen(soft(mid * 2.0, 2.0)) * 0.8, 0.01)
    # The ringing light: a bright chord in a long hall.
    ring = shimmer(8.0, [523.25, 659.25, 783.99, 1046.5, 1318.5, 2093.0], decay=3.2)
    place(out, reverb(ring, impulse_response(6.0, 10000, 3000), 0.6)[:, : int(8 * SR)] * 0.55, 0.05)
    place(out, widen(rumble(11.0, 160, 3.2)) * 1.0, 0.2)
    place(out, widen(grains(9.0, 40, 200, 2500, (0.02, 0.15), decay=2.8)) * 0.45, 0.6)
    return reverb(out, hall(), 0.3)


def eh_rim():
    """The fused rim cooling: ticking stone and a long hiss (heard where it happens)."""
    s = 9.0
    n = int(s * SR)
    tick = grains(s, 26, 1800, 7000, (0.003, 0.012)) * 2.0 * exp_decay(n, 5.0)
    hiss = filt(noise(n), "band", (2500, 9000), 1) * 0.12 * exp_decay(n, 3.5)
    groan = rumble(s, 120, 3.0) * 0.6
    return tick + hiss + groan


# ---- SS-H2 Supernova -----------------------------------------------------------------------------------------------

def sn_arm():
    """The remote wakes: crystal chimes climbing into a held chord."""
    s = 3.2
    n = int(s * SR)
    out = np.zeros(n)
    for i, f in enumerate((1318.5, 1567.98, 1975.53, 2637.0, 3135.96)):
        b = shimmer(s - i * 0.12, [f, f * 2.0], decay=1.1)
        a = int(i * 0.12 * SR)
        out[a:a + len(b)] += b * 1.2
    pad = shimmer(s, [329.63, 493.88, 659.25], decay=2.0) * env_adsr(n, 0.6, 0.3, 0.8, 0.8) * 0.6
    return reverb(widen(out + pad), impulse_response(4.0, 11000, 3000), 0.45)


def sn_ignite():
    """A star lit: a bright shimmer swelling out of nothing, a soft bloom of air."""
    s = 4.5
    n = int(s * SR)
    k = np.linspace(0, 1, n)
    stars = shimmer(s, [880, 1108.73, 1318.5, 1760, 2217.46, 2637.0], decay=9.0) * np.sin(np.pi * np.minimum(k * 1.5, 1) / 2) ** 2
    air = sweep_filter(noise(n, "pink"), "high", 4000, 1500, 1)
    air *= env_adsr(n, 1.5, 0.5, 0.4, 2.0) * 0.12
    sub = osc(55, n) * env_adsr(n, 2.0, 0.5, 0.6, 1.5) * 0.4
    return reverb(widen(stars + air + sub), impulse_response(5.0, 11000, 2500), 0.5)


def sn_swell():
    """Eight seconds of a star swelling: an endless rise, plasma boiling, its pulse throbbing harder."""
    s = 8.2
    n = int(s * SR)
    k = np.linspace(0, 1, n)
    rise = shepard_rise(s, 40.0, 7, 0.32) * (0.3 + 0.7 * k)
    rise = filt(rise, "low", 3500)
    boil = grains(s, 90, 150, 1800, (0.02, 0.1)) * (0.2 + 0.8 * k ** 2) * 0.5
    roar = sweep_filter(noise(n, "brown"), "low", 120, 900, 2) * (0.2 + 0.8 * k ** 1.5)
    pulse = np.zeros(n)
    t = 0.5
    while t < s - 0.3:
        b = sub_drop(0.7, 58, 30, 0.2, 2.0)
        a = int(t * SR)
        pulse[a:a + len(b)] += b[: n - a] * (0.4 + 0.6 * t / s)
        t += 1.25 - 0.55 * t / s
    mix = rise * 0.8 + boil + roar * 0.9 + pulse
    return cut_at(reverb(widen(soft(mix * 1.2, 1.3), 0.6), hall(), 0.28), 8.0, 0.03)


def sn_implode():
    """The giant falls in on itself: a long suck downward, then a breath of silence."""
    s = 1.9
    n = int(s * SR)
    k = np.linspace(0, 1, n)
    suck = sweep_filter(noise(n, "pink"), "low", 9000, 120, 2) * (0.4 + 0.6 * k)
    tone = osc(glide(900, 45, n, 0.7), n) * 0.5
    rev = reverb(crack(1.2, 300, 5000, 0.2), impulse_response(1.5, 5000, 1000), 0.9)[0][: int(1.4 * SR)][::-1]
    mix = suck * 0.7 + tone
    mix[-len(rev):] += rev * 1.2
    cut = int(1.45 * SR)
    mix[cut:] = 0.0
    mix[cut - 200:cut] *= np.linspace(1, 0, 200)
    return widen(mix, 0.7)


def sn_blast():
    """It goes: a crack of white, a choir of light, the deepest boom and a sky's worth of thunder."""
    s = 14.0
    out = silence(s)
    place(out, widen(crack(0.5, 2500, 14000, 0.025)) * 1.4, 0.0)
    place(out, sub_drop(5.0, 110, 20, 1.6, 2.6) * 1.6, 0.0)
    body = filt(noise(int(4 * SR), "pink"), "band", (120, 3000), 2) * exp_decay(int(4 * SR), 0.7)
    place(out, widen(soft(body * 2.2, 2.0)) * 0.9, 0.0)
    light = choir(10.0, [130.81, 196.0, 261.63, 329.63, 392.0], (650, 1080, 2650)) * env_adsr(int(10 * SR), 0.05, 1.5, 0.6, 5.0)
    place(out, reverb(light, impulse_response(6.0, 9000, 2000), 0.55)[:, : int(10 * SR)] * 0.9, 0.08)
    bells = shimmer(9.0, [1046.5, 1318.5, 1567.98, 2093.0, 2637.0, 3135.96], decay=3.5)
    place(out, reverb(bells, impulse_response(6.0, 12000, 4000), 0.6)[:, : int(9 * SR)] * 0.45, 0.1)
    place(out, widen(rumble(13.0, 140, 4.0, 0.5)) * 1.1, 0.25)
    return reverb(out, hall(), 0.3)


def sn_wave():
    """The shock front reaching you: a wall of wind and grit going past."""
    s = 4.5
    n = int(s * SR)
    k = np.linspace(0, 1, n)
    wall = sweep_filter(noise(n, "pink"), "low", 6000, 400, 2) * env_adsr(n, 0.04, 0.6, 0.35, 2.5)
    grit = grains(s, 260, 800, 7000, (0.004, 0.03)) * env_adsr(n, 0.02, 1.0, 0.2, 2.0) * 0.6
    thud = sub_drop(1.6, 70, 30, 0.4, 2.0)
    mono = soft(wall * 1.3, 1.3) + grit
    out = pan(mono, np.linspace(-0.7, 0.7, n))  # it sweeps past
    place(out, thud, 0.0)
    return reverb(out, hall(), 0.2)


def sn_burn():
    """The land burning to glass: crackle and sizzle (heard where it happens)."""
    s = 6.0
    n = int(s * SR)
    crackle = grains(s, 120, 1500, 9000, (0.002, 0.01)) * 1.6
    sizzle = filt(noise(n), "band", (3000, 11000), 2) * 0.1 * (0.7 + 0.3 * np.sin(2 * np.pi * 0.7 * np.arange(n) / SR))
    roar = filt(noise(n, "brown"), "low", 250) * 0.5
    return (crackle + sizzle + roar) * env_adsr(n, 0.3, 0.5, 0.8, 2.5)


def sn_dawn():
    """Night turned to day: a high, slow choir over everything."""
    s = 11.0
    n = int(s * SR)
    c = choir(s, [261.63, 329.63, 392.0, 493.88, 587.33], (500, 900, 2700)) * env_adsr(n, 3.0, 1.0, 0.8, 5.0)
    glow = shimmer(s, [1046.5, 1567.98, 1975.53], decay=8.0) * env_adsr(n, 4.0, 1.0, 0.7, 4.0) * 0.4
    return reverb(widen(c + glow), impulse_response(7.0, 9000, 2500), 0.55)


SOUNDS = {
    # name: (maker, loudness LUFS, subtitle)
    "eh_arm": (eh_arm, -15.0, "Event Horizon arms"),
    "eh_crush": (eh_crush, -14.0, "A satellite crushed"),
    "eh_fall": (eh_fall, -12.5, "A black hole falls"),
    "eh_land": (eh_land, -11.5, "A black hole lands"),
    "eh_feed": (eh_feed, -13.0, "A black hole feeds"),
    "eh_tear": (eh_tear, -14.0, "The ground is torn away"),
    "eh_collapse": (eh_collapse, -13.0, "A black hole collapses"),
    "eh_flash": (eh_flash, -10.5, "A black hole evaporates"),
    "eh_rim": (eh_rim, -18.0, "Molten rock cools"),
    "sn_arm": (sn_arm, -15.0, "Supernova arms"),
    "sn_ignite": (sn_ignite, -15.0, "A star ignites"),
    "sn_swell": (sn_swell, -13.0, "A star swells"),
    "sn_implode": (sn_implode, -13.0, "A star collapses"),
    "sn_blast": (sn_blast, -10.5, "A supernova"),
    "sn_wave": (sn_wave, -12.0, "The shock front passes"),
    "sn_burn": (sn_burn, -17.0, "The land burns"),
    "sn_dawn": (sn_dawn, -16.0, "Night turns to day"),
}


def main():
    folder = sys.argv[1] if len(sys.argv) > 1 else "native/data/content/assets/ss_port"
    only = sys.argv[2:] or list(SOUNDS)
    index = {}
    for name, (maker, lufs, subtitle) in SOUNDS.items():
        index[name] = {"subtitle": "subtitles.ss_port." + name, "sounds": [{"name": "ss_port:" + name, "stream": True}]}
        if name not in only:
            continue
        x = maker()
        path = write(folder, name, x, lufs)
        print(f"{name}: {x.shape[-1] / SR:.2f} s -> {path}")
    with open(os.path.join(folder, "sounds.json"), "w") as f:
        json.dump(index, f, indent=2)
    lang = {"subtitles.ss_port." + n: s for n, (_, _, s) in SOUNDS.items()}
    os.makedirs(os.path.join(folder, "lang"), exist_ok=True)
    with open(os.path.join(folder, "lang", "en_us.json"), "w") as f:
        json.dump(lang, f, indent=2)


if __name__ == "__main__":
    main()
