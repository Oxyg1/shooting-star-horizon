"""The trailer's score: 60 seconds at 120 BPM in D minor, synthesised (no
samples), its hits on the cuts of `edit.json`. Run from the repository:

    python3 tools/trailer/music.py tools/trailer/assets/music.ogg
"""
import os
import sys

import numpy as np

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "sounds"))
from synth import (  # noqa: E402
    SR, RNG, choir, crack, env_adsr, exp_decay, filt, glide, hall, impulse_response, limit, measure, noise, osc, pan,
    place, reverb, rumble, shepard_rise, shimmer, silence, soft, sub_drop, sweep_filter, widen,
)

BPM = 120
BEAT = 60 / BPM
LENGTH = 60.0
# Where the edit cuts: the intro, seven sections of six seconds, the montage, the end.
SECTIONS = [4.0 + 6.0 * i for i in range(7)]
MONTAGE = 46.0
END = 54.0

# D minor: Dm, Bb, C, A, a bar each.
CHORDS = [(38, [50, 53, 57]), (34, [46, 50, 53]), (36, [48, 52, 55]), (33, [45, 49, 52])]


def hz(midi):
    return 440.0 * 2 ** ((midi - 69) / 12)


def kick():
    n = int(0.45 * SR)
    body = osc(glide(160, 42, n, 0.35), n) * exp_decay(n, 0.16)
    click = filt(noise(n), "band", (2000, 6000)) * exp_decay(n, 0.004) * 0.6
    return soft((body + click) * 1.6, 1.8)


def snare():
    n = int(0.3 * SR)
    tone = osc(glide(210, 170, n), n) * exp_decay(n, 0.05) * 0.6
    rattle = filt(noise(n), "band", (1500, 9000)) * exp_decay(n, 0.09)
    return soft((tone + rattle) * 1.4, 1.5)


def hat(open_=False):
    n = int((0.25 if open_ else 0.06) * SR)
    return filt(noise(n), "high", 7000) * exp_decay(n, 0.08 if open_ else 0.015) * 0.5


def boom():
    s = 4.0
    out = silence(s)
    place(out, sub_drop(3.5, 85, 26, 1.0, 2.4) * 1.2, 0.0)
    place(out, widen(crack(0.6, 900, 9000, 0.06)) * 0.8, 0.0)
    place(out, widen(rumble(3.8, 200, 1.4)) * 0.6, 0.02)
    return reverb(out, hall(), 0.3)


def riser(seconds):
    n = int(seconds * SR)
    k = np.linspace(0, 1, n)
    swoosh = sweep_filter(noise(n, "pink"), "high", 200, 6000, 2) * k ** 2
    tone = shepard_rise(seconds, 55, 6, 0.9) * k ** 1.5 * 0.6
    return widen(swoosh * 0.6 + tone)


def bass_note(midi, seconds):
    n = int(seconds * SR)
    f = hz(midi)
    x = osc(f, n, "saw") * 0.7 + osc(f / 2, n) * 0.6
    x = sweep_filter(x, "low", 900, 180, 2)
    return soft(x * env_adsr(n, 0.004, 0.18, 0.5, 0.03) * 1.3, 1.4)


def pluck(midi, seconds):
    n = int(seconds * SR)
    f = hz(midi)
    x = osc(f, n, "saw") + 0.5 * osc(f * 2.005, n, "saw")
    x = sweep_filter(x, "low", 5000, 700, 2) * env_adsr(n, 0.002, 0.12, 0.0, 0.02)
    return filt(x, "high", 250) * 0.5


def pad(midis, seconds):
    n = int(seconds * SR)
    x = np.zeros(n)
    for m in midis:
        for d in (-0.12, 0.0, 0.11):
            x += osc(hz(m) * 2 ** (d / 12), n, "saw", phase=RNG.random() * 6)
    x = filt(x / (len(midis) * 3), "low", 1800)
    return x * env_adsr(n, 0.4, 0.3, 0.8, 0.5)


def build():
    total = silence(LENGTH)
    # Intro: a dark pad swelling, a riser into the first cut.
    intro_pad = pad([50, 53, 57, 62], 4.2) * np.linspace(0.2, 1.0, int(4.2 * SR))
    place(total, widen(intro_pad) * 0.5, 0.0)
    place(total, reverb(shimmer(4.0, [hz(74), hz(81), hz(86)], 2.5), impulse_response(4.0, 11000, 3000), 0.6) * 0.25, 0.3)
    place(total, riser(2.5) * 0.7, 1.5)

    # The sections and the montage: a beat, a bass line, an arpeggio, the chords.
    beat_at = 4.0
    bar = 0
    while beat_at < END - 1e-6:
        montage = beat_at >= MONTAGE
        root, chord = CHORDS[bar % 4]
        bar_start = beat_at
        # The pad, a bar long.
        place(total, widen(pad(chord, 4 * BEAT + 0.4)) * (0.32 if not montage else 0.4), bar_start)
        for b in range(4):
            t = bar_start + b * BEAT
            place(total, kick() * 0.95, t)
            if montage:
                place(total, kick() * 0.6, t + BEAT / 2)
            if b in (1, 3):
                place(total, widen(snare(), 0.3) * 0.7, t)
            if montage:
                place(total, widen(snare(), 0.3) * 0.35, t + BEAT / 2)
            for h in range(2 if not montage else 4):
                place(total, pan(hat(h == 1 and not montage), 0.3 if h % 2 else -0.3) * 0.55, t + h * BEAT / (2 if not montage else 4))
            # Bass in eighths.
            for e in range(2):
                place(total, bass_note(root if e == 0 else root + 12, BEAT / 2) * 0.55, t + e * BEAT / 2)
            # Arpeggio in sixteenths over the chord.
            for s in range(4):
                m = chord[(b * 4 + s) % len(chord)] + (12 if s % 2 else 24)
                arp = pluck(m, BEAT / 4 + 0.05)
                place(total, pan(arp, 0.5 if s % 2 else -0.5) * (0.35 if not montage else 0.45), t + s * BEAT / 4)
        beat_at += 4 * BEAT
        bar += 1

    # The hits on the cuts.
    for at in SECTIONS + [MONTAGE]:
        place(total, boom() * 0.85, at)
    for k in range(1, 4):
        place(total, boom() * 0.45, MONTAGE + 2.0 * k)
    # Builds before the montage and the end.
    place(total, riser(2.0) * 0.6, MONTAGE - 2.0)
    place(total, riser(2.0) * 0.7, END - 2.0)

    # The end: the deepest hit, a major chord sung out, and silence.
    place(total, boom() * 1.1, END)
    end_choir = choir(6.0, [hz(50), hz(57), hz(62), hz(66), hz(69)], (650, 1080, 2650)) * env_adsr(int(6 * SR), 0.05, 1.0, 0.7, 3.5)
    place(total, reverb(end_choir, impulse_response(5.0, 9000, 2500), 0.5)[:, : int(6 * SR)] * 0.6, END + 0.05)
    n = total.shape[1]
    fade = np.ones(n)
    fade[int((LENGTH - 1.5) * SR):] = np.linspace(1, 0, n - int((LENGTH - 1.5) * SR)) ** 2
    return total * fade


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "tools/trailer/assets/music.ogg"
    x = build()
    x = x / (np.max(np.abs(x)) + 1e-12) * 0.5
    for _ in range(3):
        x = limit(x * 10 ** ((-12.0 - measure(x)) / 20), -1.0)
    import subprocess
    import tempfile
    pcm = (np.clip(x.T, -1, 1) * 32767).astype("<i2")
    with tempfile.NamedTemporaryFile(suffix=".raw", delete=False) as f:
        f.write(pcm.tobytes())
        raw = f.name
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "s16le", "-ar", str(SR), "-ac", "2", "-i", raw,
                    "-c:a", "libvorbis", "-q:a", "6", out], check=True)
    os.unlink(raw)
    print(out, f"{x.shape[1] / SR:.1f} s")


if __name__ == "__main__":
    main()
