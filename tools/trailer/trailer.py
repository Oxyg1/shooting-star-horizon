"""Films the mod's trailer and cuts it, on the player's own computer.

1. It starts recording the screen (ffmpeg: the GPU's desktop capture and
   encoder where there are, plain capture else).
2. In the game, in a new world made for it, type `/tss trailer`: the mod's
   director casts every skill in turn, plays each film, hides the HUD and
   writes when each moment happened (`shooting_star_trailer.jsonl`).
3. When the director is done, the recording stops and the trailer is cut by
   `edit.json` to `assets/music.ogg`, with the titles of `assets/`.

    python trailer.py                      record, then cut
    python trailer.py --cut RAW MARKS      only cut a recording already made

Everything goes to `trailer-out/` beside this file (or `--out`).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ASSETS = HERE / "assets"
MARKS_NAME = "shooting_star_trailer.jsonl"
TICK_MS = 50.0


def log(text):
    print(f"[trailer] {text}", flush=True)


def ffmpeg():
    exe = shutil.which("ffmpeg")
    if not exe:
        sys.exit("ffmpeg was not found. Run trailer.ps1 (it installs it), or install ffmpeg and try again.")
    return exe


def run(args, quiet=True):
    cmd = [ffmpeg(), "-hide_banner", "-y"] + (["-loglevel", "error"] if quiet else []) + [str(a) for a in args]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        sys.stderr.write(result.stderr[-4000:])
        raise SystemExit(f"ffmpeg failed: {' '.join(cmd[:12])} ...")


# ---- the marks --------------------------------------------------------------------------------------------------

def read_marks(path):
    events = []
    for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if line:
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return events


class Clock:
    """Each skill's ticks in wall-clock milliseconds, from its marks."""

    def __init__(self, events):
        self.ages = {}
        for e in events:
            if e.get("event") in ("cast", "age") and e.get("skill"):
                self.ages.setdefault(e["skill"], []).append((int(e.get("age", 0)), float(e["ms"])))
        for skill, points in self.ages.items():
            # The newest cast of a skill only, if it was cast twice.
            casts = [i for i, (a, _) in enumerate(points) if a == 0]
            start = casts[-1] if casts else 0
            self.ages[skill] = sorted(set(points[start:]))
        self.start = next((float(e["ms"]) for e in events if e.get("event") == "start"), None)
        self.end = next((float(e["ms"]) for e in reversed(events) if e.get("event") == "end"), None)

    def has(self, skill):
        return bool(self.ages.get(skill))

    def ms(self, skill, age):
        points = self.ages.get(skill)
        if not points:
            return None
        if len(points) == 1:
            return points[0][1] + (age - points[0][0]) * TICK_MS
        # Between the two nearest marks; past them, at their pace.
        for (a0, m0), (a1, m1) in zip(points, points[1:]):
            if a0 <= age <= a1:
                return m0 + (m1 - m0) * (age - a0) / max(a1 - a0, 1)
        (a0, m0), (a1, m1) = (points[0], points[1]) if age < points[0][0] else (points[-2], points[-1])
        pace = (m1 - m0) / max(a1 - a0, 1)
        return m0 + (age - a0) * pace


# ---- recording --------------------------------------------------------------------------------------------------

def game_folders():
    """Folders that hold `horizon-mods/` (Horizon's game folders), found once."""
    roots = []
    for var in ("APPDATA", "LOCALAPPDATA"):
        if os.environ.get(var):
            roots.append(Path(os.environ[var]))
    roots.append(Path.home())
    found = []
    for root in roots:
        for base in (root / "horizon", root / ".horizon", root / "Horizon", root / "horizon-launcher"):
            if not base.is_dir():
                continue
            for dirpath, dirnames, _ in os.walk(base):
                depth = Path(dirpath).relative_to(base).parts
                if len(depth) > 6:
                    dirnames[:] = []
                    continue
                if "horizon-mods" in dirnames:
                    found.append(Path(dirpath))
                dirnames[:] = [d for d in dirnames if not d.startswith(".") and d not in ("saves", "logs", "resourcepacks", "shaderpacks", "source", "target")]
    return found


class Recorder:
    def __init__(self, path, audio=None):
        self.path = path
        self.audio = audio
        self.proc = None
        self.start_ms = None
        self.lines = []

    def _command(self, gpu):
        video = (["-f", "lavfi", "-i", "ddagrab=output_idx=0:framerate=60:draw_mouse=0"] if gpu
                 else ["-f", "gdigrab", "-framerate", "60", "-draw_mouse", "0", "-i", "desktop"])
        audio = ["-f", "dshow", "-i", f"audio={self.audio}"] if self.audio else []
        codec = (["-c:v", "h264_nvenc", "-preset", "p5", "-cq", "18", "-g", "120"] if gpu
                 else ["-c:v", "libx264", "-preset", "ultrafast", "-crf", "18", "-pix_fmt", "yuv420p"])
        acodec = ["-c:a", "aac", "-b:a", "192k"] if self.audio else ["-an"]
        return [ffmpeg(), "-hide_banner", "-y", "-stats"] + video + audio + codec + acodec + [str(self.path)]

    def start(self):
        for gpu in (True, False):
            cmd = self._command(gpu)
            self.proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
            threading.Thread(target=self._read, daemon=True).start()
            for _ in range(60):
                time.sleep(0.1)
                if self.start_ms is not None or self.proc.poll() is not None:
                    break
            if self.proc.poll() is None:
                log("recording the screen" + (" (GPU capture)" if gpu else " (plain capture: slower)"))
                return
            log("GPU capture is not available here; trying plain capture" if gpu else "recording failed")
            sys.stderr.write("".join(self.lines[-15:]))
        raise SystemExit("could not record the screen")

    def _read(self):
        # The wall-clock time of the first frame: now, less the time ffmpeg says it has recorded.
        for line in iter(self.proc.stderr.readline, ""):
            self.lines.append(line)
            m = re.search(r"time=(\d+):(\d+):(\d+\.\d+)", line)
            if m:
                recorded = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3))
                at = time.time() * 1000 - recorded * 1000
                self.start_ms = at if self.start_ms is None else min(self.start_ms, at)

    def stop(self):
        if self.proc and self.proc.poll() is None:
            try:
                self.proc.stdin.write("q")
                self.proc.stdin.flush()
            except OSError:
                pass
            try:
                self.proc.wait(30)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        log("recording stopped")


def record(out, audio=None):
    folders = game_folders()
    if not folders:
        log("no Horizon game folder found: the marks will be looked for once they appear")
    began = time.time()
    raw = out / "raw.mkv"
    rec = Recorder(raw, audio)
    rec.start()
    log("now, in the game (a new world made for the trailer), type:  /tss trailer")
    log("leave the mouse and keyboard alone until it says the trailer is done (about four minutes)")
    marks = None
    try:
        while True:
            time.sleep(1.0)
            if marks is None:
                for folder in folders or game_folders():
                    candidate = folder / MARKS_NAME
                    if candidate.is_file() and candidate.stat().st_mtime >= began:
                        marks = candidate
                        log(f"the director started ({marks})")
                        break
                continue
            if any(e.get("event") == "end" for e in read_marks(marks)):
                log("the director is done")
                time.sleep(3.0)
                break
            if rec.proc.poll() is not None:
                raise SystemExit("the recording stopped by itself")
    except KeyboardInterrupt:
        log("stopped by hand")
    finally:
        rec.stop()
    if marks is None:
        raise SystemExit("no marks were written: was /tss trailer typed in the game?")
    shutil.copy(marks, out / MARKS_NAME)
    (out / "record.json").write_text(json.dumps({"start_ms": rec.start_ms}), encoding="utf-8")
    return raw, out / MARKS_NAME, rec.start_ms


# ---- the cut ----------------------------------------------------------------------------------------------------

def has_audio(path):
    probe = shutil.which("ffprobe")
    if not probe:
        return False
    out = subprocess.run([probe, "-v", "error", "-select_streams", "a", "-show_entries", "stream=index", "-of", "csv=p=0", str(path)],
                         capture_output=True, text=True).stdout
    return bool(out.strip())


def cut(raw, marks_path, start_ms, out, offset=0.0):
    plan = json.loads((HERE / "edit.json").read_text(encoding="utf-8"))
    w, h = plan["size"]
    fps = plan["fps"]
    clock = Clock(read_marks(marks_path))
    if start_ms is None:
        # Without the recorder's clock: the director's start, as near as it can be told.
        start_ms = clock.start
        log("no recording clock: assuming the recording began as the director did")
    audio = has_audio(raw)
    work = out / "cut"
    work.mkdir(parents=True, exist_ok=True)
    vf = f"scale={w}:{h}:force_original_aspect_ratio=increase:flags=lanczos,crop={w}:{h},fps={fps},format=yuv420p,setsar=1"
    enc = ["-c:v", "libx264", "-preset", "veryfast", "-crf", "16"]
    aenc = ["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2"]

    def blank(name, seconds):
        path = work / name
        args = ["-f", "lavfi", "-i", f"color=c=black:s={w}x{h}:r={fps}:d={seconds}"]
        if audio:
            args += ["-f", "lavfi", "-i", f"anullsrc=r=48000:cl=stereo", "-t", seconds]
        run(args + ["-vf", "format=yuv420p,setsar=1"] + enc + (aenc if audio else []) + ["-t", seconds, path])
        return path

    def shot(name, skill, age, seconds, extra_vf=""):
        at = clock.ms(skill, age)
        if at is None:
            log(f"{skill} was not filmed: black in its place")
            return blank(name, seconds)
        ss = max(0.0, (at - start_ms) / 1000.0 + offset)
        path = work / name
        run(["-ss", f"{ss:.3f}", "-i", raw, "-t", seconds, "-vf", vf + extra_vf] + enc
            + (aenc if audio else ["-an"]) + ["-t", seconds, path])
        return path

    pieces = [blank("000_intro.mp4", plan["intro"]["seconds"])]
    overlays = [(ASSETS / plan["intro"]["overlay"], 0.0)]
    cuts = []
    t = plan["intro"]["seconds"]
    n = 1
    for section in plan["sections"]:
        overlays.append((ASSETS / section["card"], t + 0.15))
        for age, seconds in section["shots"]:
            pieces.append(shot(f"{n:03d}.mp4", section["skill"], age, seconds))
            cuts.append(t)
            t += seconds
            n += 1
    montage_start = t
    for k, (skill, age, seconds) in enumerate(plan["montage"]):
        pieces.append(shot(f"{n:03d}.mp4", skill, age, seconds))
        if k % 2 == 0:
            cuts.append(t)
        t += seconds
        n += 1
    outro = plan["outro"]
    under_skill, under_age = outro["under"]
    pieces.append(shot(f"{n:03d}_outro.mp4", under_skill, under_age, outro["seconds"], ",eq=brightness=-0.12:saturation=0.8,gblur=sigma=6"))
    overlays.append((ASSETS / outro["overlay"], t))
    t += outro["seconds"]
    log(f"{len(pieces)} pieces cut, {t:.1f} s")

    listing = work / "pieces.txt"
    listing.write_text("".join(f"file '{p.name}'\n" for p in pieces), encoding="utf-8")
    body = work / "body.mp4"
    run(["-f", "concat", "-safe", "0", "-i", listing, "-c", "copy", body])

    # The titles, the flashes on the cuts and the rings, over the body; the score under it.
    inputs = ["-i", body, "-i", ASSETS / "music.ogg"]
    graph = []
    last = "0:v"
    index = 2
    flashes = [c for c in cuts if c > 0.1 and c != montage_start]
    layers = [(p, at) for p, at in overlays] + [(ASSETS / "flash.webm", c - 0.05) for c in flashes] + [(ASSETS / "impact_ring.webm", r) for r in plan["rings"]]
    layers += [(ASSETS / "flash.webm", montage_start - 0.05)]
    for k, (path, at) in enumerate(layers):
        if not path.is_file():
            log(f"missing {path.name}: left out")
            continue
        inputs += ["-c:v", "libvpx-vp9", "-i", path]
        graph.append(f"[{index}:v]format=yuva420p,setpts=PTS-STARTPTS+{at:.3f}/TB[o{k}]")
        graph.append(f"[{last}][o{k}]overlay=eof_action=pass:format=auto[v{k}]")
        last = f"v{k}"
        index += 1
    length = plan["length"]
    graph.append(f"[{last}]fade=t=out:st={length - 0.8:.2f}:d=0.8,format=yuv420p[vout]")
    if audio:
        graph.append(f"[0:a]volume={plan['game_audio']}[game];[1:a][game]amix=inputs=2:duration=first:normalize=0,alimiter=limit=0.95[aout]")
    else:
        graph.append("[1:a]anull[aout]")
    final = out / "shooting-star-trailer.mp4"
    run(inputs + ["-filter_complex", ";".join(graph), "-map", "[vout]", "-map", "[aout]", "-t", length,
                  "-c:v", "libx264", "-preset", "slow", "-crf", "17", "-pix_fmt", "yuv420p",
                  "-c:a", "aac", "-b:a", "256k", "-movflags", "+faststart", final])
    log(f"the trailer is ready: {final}")
    return final


def main():
    parser = argparse.ArgumentParser(description="Film and cut the trailer of The Shooting Star for Horizon.")
    parser.add_argument("--out", default=str(HERE / "trailer-out"), help="where the recording and the trailer go")
    parser.add_argument("--audio", default=None, help="a DirectShow audio device to record the game's sound from (e.g. \"Stereo Mix (Realtek Audio)\")")
    parser.add_argument("--offset", type=float, default=0.0, help="seconds to shift every cut by, if they land early (+) or late (-)")
    parser.add_argument("--cut", nargs=2, metavar=("RAW", "MARKS"), help="only cut: a recording already made and its marks")
    args = parser.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    if args.cut:
        raw, marks = Path(args.cut[0]), Path(args.cut[1])
        saved = Path(raw).with_name("record.json")
        start_ms = json.loads(saved.read_text())["start_ms"] if saved.is_file() else None
    else:
        raw, marks, start_ms = record(out, args.audio)
    cut(raw, marks, start_ms, out, args.offset)


if __name__ == "__main__":
    main()
