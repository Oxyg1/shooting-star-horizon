// SS-05 Tesseract's light: the original's `TesseractFx` (1.4.4, whose
// tess_world and tess_sky shaders 1.5.2 keeps), its square's geometry, the
// cubes' schedule, the hypercube turning over the square and the engine in
// the sky, the screen's blows, and its film's camera.
//
// The square and the cubes are the server's (`shooting_star_spells`): the
// same origin, cell, pit floors, heights and lifts, from the same seed.
use super::shooting_star_cutscenes::{self as cs, Ground, Pose, Shot, Subject, V};
use super::shooting_star_films as films;
use super::shooting_star_skills::{self as sk, hash};
use horizon_client_render::gfx::native_render::shooting_star_fxframe::{Frame, Pass, Screen};

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

fn smooth(x: f32) -> f32 {
    let x = clamp01(x);
    x * x * (3.0 - 2.0 * x)
}

fn window(t: f32, in0: f32, in1: f32, out0: f32, out1: f32) -> f32 {
    clamp01((t - in0) / (in1 - in0).max(0.001)) * (1.0 - clamp01((t - out0) / (out1 - out0).max(0.001)))
}

/// The ring a cell is in (0 the middle four, 3 round the edge).
fn ring(x: i32, z: i32) -> i32 {
    sk::tess_ring(x, z)
}

/// The tick each of the 64 cubes lifts (`k()`): ring by ring from the
/// middle, the outer two rings two cubes a tick. Indexed `z * 8 + x`.
pub fn cube_ticks() -> [i32; 64] {
    let mut out = [0; 64];
    let mut seen = [0; 4];
    for z in 0..8 {
        for x in 0..8 {
            let r = ring(x, z);
            let n = seen[r as usize];
            seen[r as usize] += 1;
            out[(z * 8 + x) as usize] = 474 + r * 20 + if r < 2 { n } else { n / 2 };
        }
    }
    out
}

/// The square as the spell lays it out.
#[derive(Clone, Copy, Debug)]
pub struct Square {
    /// The spell's scale (`power / 0.4`).
    pub scale: f64,
    pub zone: i32,
    pub cell: i32,
    /// West and north edges.
    pub ox: i32,
    pub oz: i32,
    /// The target's ground (`T`).
    pub ground: i32,
    /// The cubes' bottom (`U`): a cell under the ground.
    pub bottom: i32,
    pub min_y: i32,
    pub max_y: i32,
    pub seed: u64,
}

impl Square {
    pub fn new(start: &sk::Start, min_y: i32, max_y: i32) -> Square {
        let scale = sk::tess_scale(start.power);
        let zone = sk::tess_zone(scale);
        let cell = zone / 8;
        let ground = start.target[1].floor() as i32;
        Square {
            scale,
            zone,
            cell,
            ox: sk::tess_origin(start.target[0], zone),
            oz: sk::tess_origin(start.target[2], zone),
            ground,
            bottom: (ground - 1 - cell).max(min_y + 1),
            min_y,
            max_y,
            seed: start.seed,
        }
    }

    /// The square's middle on the ground (`i()`).
    pub fn centre(&self) -> V {
        [f64::from(self.ox) + f64::from(self.zone) * 0.5, f64::from(self.ground), f64::from(self.oz) + f64::from(self.zone) * 0.5]
    }

    /// The pit's floor under cell (x, z) once its cube is gone (`W`).
    pub fn floor(&self, x: i32, z: i32) -> i32 {
        let depth = (4 - ring(x, z)) * (20.0 * self.scale).round() as i32;
        (self.bottom - depth).max(self.bottom.min(self.min_y + 4))
    }

    /// The engine's half size (`f(y)`).
    fn engine_half(&self) -> i32 {
        (13.0 * self.scale.clamp(0.5, 2.0)).round() as i32
    }

    /// A cube in the sky: its bottom, height and side (`b()`), as the
    /// server lifts it (`tess_lift`, kept under the sky).
    pub fn cube(&self, x: i32, z: i32) -> (i32, i32, i32) {
        let land = (self.ground + 1 - self.bottom).min(self.max_y - self.bottom);
        let raw = {
            let v = hash(self.seed, i64::from(x * 7 + 3), i64::from(z * 13 + 5));
            if v < 0.42 {
                0
            } else if v < 0.62 {
                1
            } else if v < 0.8 {
                2
            } else {
                3
            }
        };
        let kind = if raw != 3 || (land <= self.cell + (16.0 * self.scale).round() as i32 && self.cell + land <= self.max_y - 2 - self.bottom) { raw } else { 2 };
        let height = if kind == 3 { self.cell } else { land };
        let room = self.max_y - 2 - self.bottom - height;
        let least = self.cell + (40.0 * self.scale).round() as i32;
        let (base, height) = if room >= least {
            (self.bottom + sk::tess_lift(self.seed, x, z, self.scale).min(room).max(least), height)
        } else if room >= 12 {
            (self.bottom + room, height)
        } else {
            let base = self.bottom + 12;
            (base, (self.max_y - 2 - base).clamp(0, height))
        };
        (base, height, if kind == 3 { land.min(self.cell) } else { self.cell })
    }

    /// The engine's height (`ag`): over the middle cubes once they are up.
    pub fn engine_y(&self, t: f32) -> i32 {
        let first = self.bottom + self.cell;
        let half = self.engine_half();
        let mut high = (first + 150).min(self.max_y - 30);
        if t >= 461.0 {
            let lowest = (0..64).filter(|i| ring(i % 8, i / 8) == 0).map(|i| self.cube(i % 8, i / 8).0).min().unwrap_or(i32::MAX);
            high = high.min(lowest - half - 8);
        }
        high.max(first + half + 8)
    }

    /// The engine's middle (`s()`).
    pub fn engine(&self, t: f32) -> V {
        let c = self.centre();
        [c[0] + 0.5, f64::from(self.engine_y(t)) + 0.5, c[2] + 0.5]
    }
}

/// One corner of the hypercube, turned through four angles and projected
/// (`TessPath.vertex`): x, y, z and its depth in w.
pub fn hyper(i: usize, a: [f64; 4]) -> [f64; 4] {
    let s = |bit: usize| if i & bit == 0 { -1.0 } else { 1.0 };
    let (x, y, z, w) = (s(1), s(2), s(4), s(8));
    let (c, n) = (a[0].cos(), a[0].sin());
    let (x1, w1) = (x * c - w * n, x * n + w * c);
    let (c, n) = (a[1].cos(), a[1].sin());
    let (y1, w2) = (y * c - w1 * n, y * n + w1 * c);
    let (c, n) = (a[2].cos(), a[2].sin());
    let (z1, w3) = (z * c - w2 * n, z * n + w2 * c);
    let k = 3.2 / (3.2 - w3);
    let (px, py, pz) = (x1 * k * 1.1, y1 * k * 1.1, z1 * k * 1.1);
    let (c, n) = (a[3].cos(), a[3].sin());
    [px * c - pz * n, py, px * n + pz * c, w3]
}

/// The hypercube's four turns at tick `t`, slowing to a stop at the blow.
fn turns(t: f32) -> [f64; 4] {
    let s = f64::from(t) / 20.0;
    let k = 1.0 - f64::from(smooth((t - 562.0) / 20.0));
    [s * 0.9 * k, s * 0.55 * k, s * 0.3 * k, s * 0.18 * k]
}

/// The countdown's pulses (`pulsePhase`, 13.05 s over 261 ticks).
fn pulses(t: f32) -> f32 {
    if t < 29.0 {
        return 0.0;
    }
    let s = f64::from((t - 29.0).min(261.0)) / 20.0;
    let k = s / 13.05;
    (0.6 * s + 3.4 * 13.05 * k * k * k / 3.0) as f32
}

/// The square's light: the ground's grid and the cubes (tess_world), and
/// the sky scene over it (tess_sky).
pub fn passes(frame: &mut Frame, sq: &Square, t: f32, seed_f: f32, camera: V) {
    if (280.0..=466.0).contains(&t) {
        sky(frame, sq, t, camera);
    }
    if !(8.0..=1262.0).contains(&t) {
        return;
    }
    let engine = sq.engine(t);
    let a = turns(t);
    let size = (f64::from(smooth((t - 562.0) / 24.0)) * (8.125 - 30.0) + 30.0) * sq.scale.clamp(0.5, 2.0);
    let open = f64::from(smooth((t - 426.0) / 6.0));
    let pts: Vec<[f32; 4]> = (0..16)
        .map(|i| {
            let v = hyper(i, a);
            [
                (engine[0] + v[0] * size * open - camera[0]) as f32,
                (engine[1] + v[1] * size * open - camera[1]) as f32,
                (engine[2] + v[2] * size * open - camera[2]) as f32,
                v[3] as f32,
            ]
        })
        .collect();
    let ticks = cube_ticks();
    let cubes: Vec<[f32; 4]> = (0..64)
        .map(|i| {
            let (x, z) = (i as i32 % 8, i as i32 / 8);
            if t >= ticks[i] as f32 {
                let (base, height, side) = sq.cube(x, z);
                [(t - ticks[i] as f32) / 20.0, (f64::from(base) - camera[1]) as f32, height as f32, side as f32]
            } else {
                [-1.0, 0.0, 0.0, sq.cell as f32]
            }
        })
        .collect();
    let since = |at: f32| if t >= at { (t - at) / 20.0 } else { -1.0 };
    let after = if t < 562.0 { 1.0 } else { (1.0 + (0.4 - 1.0) * smooth((t - 562.0) / 100.0)) * (1.0 - smooth((t - 862.0) / 300.0)) };
    frame.passes.push(
        Pass::new("tess_world")
            .vec4s("Pts", pts)
            .vec4s("Cubes", cubes)
            .at("Grid", [f64::from(sq.ox), f64::from(sq.ground), f64::from(sq.oz)])
            .f("P0", &[t / 20.0, seed_f, window(t, 9.0, 10.5, 962.0, 1162.0), smooth((t - 426.0) / 8.0)])
            .f("P1", &[since(438.0), since(462.0), since(562.0), pulses(t)])
            .f("P2", &[(f64::from(sq.bottom) - camera[1]) as f32, window(t, 428.0, 448.0, 562.0, 582.0), smooth((t - 562.0) / 24.0), after])
            .f("P3", &[(engine[1] - camera[1]) as f32, sq.zone as f32, window(t, 422.0, 452.0, 722.0, 882.0), sq.scale as f32])
            .f("P4", &[smooth((t - 426.0 + 1.0) / 2.0), since(426.0), (20.0 * sq.scale).round() as f32, (f64::from(sq.min_y + 4) - camera[1]) as f32]),
    );
}

/// The engine coming down from orbit (tess_sky): 1.4.4's own numbers, at
/// this spell's scale.
fn sky(frame: &mut Frame, sq: &Square, t: f32, camera: V) {
    let Some(table) = films::table("tess_sky") else { return };
    let mut sky = [0.0f64; 5];
    let mut eng = Vec::new();
    for (name, v) in table.sample(t) {
        match name {
            "Sky" if v.len() == 5 => {
                for (s, x) in sky.iter_mut().zip(&v) {
                    *s = f64::from(*x);
                }
            }
            "Eng" => eng = v,
            _ => {}
        }
    }
    // Past 398 the engine shrinks to the square's own size: the table is at scale 1.
    if t >= 398.0 {
        let k = f64::from(clamp01((t - 398.0) / 28.0)).powi(2);
        let to = (128.0 * sq.scale / 6_371_000.0).log10();
        sky[0] = 10f64.powf(3.2f64.log10() + (to - 3.2f64.log10()) * k) * 6_371_000.0;
        sky[1] = 10f64.powf(1.6f64.log10() + (to - 1.6f64.log10()) * k) * 6_371_000.0;
    }
    let c = sq.centre();
    let rel = |dy: f64| [(c[0] - camera[0]) as f32, (f64::from(sq.ground) + dy - camera[1]) as f32, (c[2] - camera[2]) as f32];
    let engine = rel(sky[0]);
    let mouth = rel(sky[3]);
    frame.passes.push(
        Pass::new("tess_sky")
            .texture("MilkyTop", "shooting_star/milky_way_top.jpg")
            .f("S0", &[t / 20.0, window(t, 284.0, 312.0, 436.0, 466.0), sky[2] as f32, window(t, 280.0, 284.0, 286.0, 298.0)])
            .f("MouthC", &[mouth[0], mouth[1], mouth[2], sky[4] as f32])
            .f("EngC", &[engine[0], engine[1], engine[2], sky[1] as f32])
            .f("EngX", &[1.0, 0.0, 0.0])
            .f("EngY", &[0.0, 1.0, 0.0])
            .f("EngZ", &[0.0, 0.0, 1.0])
            .f("KeyDir", &[-0.42, 0.82, 0.39])
            .f("KeyCol", &[2.2, 2.09, 1.94])
            .vec4s("Eng", eng.chunks(4).map(|q| [q[0], q[1], q[2], q[3]]).collect()),
    );
}

const PRISM: [u32; 7] = [0xFF242C, 0xFF8010, 0xFFE41E, 0x28EC54, 0x1C84FF, 0x5430FF, 0xBA3CFF];
const DRAWING: f32 = 1.0 / 12.0;
const OPEN: [i32; 2] = [33, 35];
const SEVER: [i32; 6] = [30, 35, 34, 30, 27, 32];
const WAVES: [i32; 4] = [33, 27, 35, 31];
const FOLD: [i32; 10] = [35, 33, 27, 34, 30, 29, 32, 31, 28, 35];

fn impacts(screen: &mut Screen, since: f32, strength: f32, modes: &[i32]) {
    if since < 0.0 || strength <= 0.0 {
        return;
    }
    if let Some(mode) = modes.get((since / DRAWING) as usize) {
        screen.add_impact(strength, *mode, 0.5, 0.5);
    }
}

fn hold_frames(screen: &mut Screen, k: f32) {
    if k > 0.001 {
        screen.desaturate = screen.desaturate.max(k);
        screen.grade[0] = screen.grade[0].max(0.3 * k);
        screen.grade[3] = screen.grade[3].max(k);
        screen.grade[1] = screen.grade[1].max(0.35 * k);
    }
}

/// A wave of cubes leaving: a flash and anime frames (`TesseractFx.wave`).
fn wave(screen: &mut Screen, since: f32, modes: &[i32], turn: usize, near: f32, gain: f32) {
    if !(0.0..2.5).contains(&since) {
        return;
    }
    if since < 0.05 {
        screen.add_flash(0xFFFFFF, 4.5 * near * gain);
    } else if since - 0.05 < modes.len() as f32 * DRAWING {
        let order: Vec<i32> = (0..modes.len()).map(|i| modes[(i + turn) % modes.len()]).collect();
        impacts(screen, since - 0.05, near, &order);
        screen.step(12.0);
        let drawing = ((since - 0.05) / DRAWING) as i32;
        screen.zoom_blur = screen.zoom_blur.max(if drawing % 2 == 0 { 0.9 } else { 0.25 } * near);
    }
    screen.add_flash(PRISM[(since * 9.0 + turn as f32 * 2.0) as usize % 7], (-since * 5.0).exp() * 0.8 * near);
    screen.shake = screen.shake.max(window(since, 0.0, 0.02, 0.5, 2.0) * 1.2 * near * gain);
    screen.aberration = screen.aberration.max(window(since, 0.0, 0.02, 0.4, 1.5) * 1.2 * near);
    screen.bloom = screen.bloom.max(0.75 * window(since, 0.0, 0.1, 1.0, 2.5));
}

/// The screen's share of the Tesseract (`TesseractFx.screen`), `near` how
/// close the camera is to the square's edge.
pub fn screen(screen: &mut Screen, sq: &Square, t: f32, near: f32) {
    let mark = (t - 9.0) / 20.0;
    if (0.0..1.0).contains(&mark) {
        screen.add_flash(0xE8F8FF, (-mark * 6.0).exp() * 0.2 * near);
        screen.aberration = screen.aberration.max((-mark * 4.0).exp() * 0.35 * near);
    }
    let lock = window(t, 280.0, 286.0, 292.0, 312.0);
    screen.aberration = screen.aberration.max(lock * 0.7 * near);
    screen.shake = screen.shake.max(lock * 0.25 * near);
    let hum = window(t, 288.0, 308.0, 394.0, 398.0);
    screen.shake = screen.shake.max(hum * 0.12 * near);
    let drop = ((t - 398.0) / 28.0).clamp(0.0, 1.0);
    if (398.0..426.0).contains(&t) {
        screen.zoom_blur = screen.zoom_blur.max(drop * drop * 0.8 * near);
        screen.aberration = screen.aberration.max(drop * 1.2 * near);
        screen.shake = screen.shake.max((0.2 + drop * drop * 0.9) * near);
    }
    hold_frames(screen, window(t, 423.0, 424.0, 425.0, 426.0) * 0.6);
    let land = (t - 426.0) / 20.0;
    if (0.0..2.5).contains(&land) {
        if land < 0.05 {
            screen.add_flash(0xFFFFFF, 5.0 * near);
        } else if land - 0.05 < OPEN.len() as f32 * DRAWING {
            impacts(screen, land - 0.05, near, &OPEN);
            screen.step(12.0);
        }
        screen.add_flash(PRISM[(land * 9.0) as usize % 7], (-land * 5.0).exp() * 0.7 * near);
        screen.shake = screen.shake.max(window(land, 0.0, 0.02, 0.6, 2.0) * 1.5 * near);
        screen.bloom = screen.bloom.max(0.8 * window(land, 0.0, 0.1, 0.8, 2.5));
    }
    let sever = window(t, 438.0, 442.0, 460.0, 462.0);
    screen.shake = screen.shake.max(sever * (0.2 + 0.4 * (t - 438.0) / 24.0) * near);
    screen.aberration = screen.aberration.max(sever * 0.5 * near);
    screen.zoom_blur = screen.zoom_blur.max(sever * sever * 0.25 * near);
    hold_frames(screen, window(t, 457.0, 459.0, 461.0, 462.0) * 0.6);
    wave(screen, (t - 462.0) / 20.0, &SEVER, 0, near, 1.0);
    for n in 0..4 {
        wave(screen, (t - 474.0 - n as f32 * 20.0) / 20.0, &WAVES, n, near, 0.7 * (0.9 + 0.1 * n as f32));
    }
    let rise = window(t, 474.0, 480.0, 552.0, 562.0);
    screen.shake = screen.shake.max(rise * 0.2 * near);
    screen.bloom = screen.bloom.max(rise * 0.35);
    hold_frames(screen, window(t, 554.0, 557.0, 561.0, 562.0) * 0.7);
    let fold = (t - 562.0) / 20.0;
    if (0.0..8.0).contains(&fold) {
        if fold < 0.06 {
            screen.add_flash(0xFFFFFF, 6.5);
        } else if fold - 0.06 < FOLD.len() as f32 * DRAWING {
            impacts(screen, fold - 0.06, 1.0, &FOLD);
            screen.step(12.0);
            let drawing = ((fold - 0.06) / DRAWING) as i32;
            screen.zoom_blur = screen.zoom_blur.max(if drawing % 2 == 0 { 1.0 } else { 0.3 });
            screen.aberration = screen.aberration.max(if drawing % 2 == 0 { 0.5 } else { 1.8 });
        }
        screen.add_flash(PRISM[(fold * 5.0) as usize % 7], (-fold * 2.5).exp() * 0.9);
        screen.shake = screen.shake.max(window(fold, 0.0, 0.02, 1.0, 1.8) * 1.9);
        screen.bloom = screen.bloom.max(1.1 * window(fold, 0.0, 0.1, 3.0, 8.0));
        screen.bloom_radius = screen.bloom_radius.max(1.0 + 1.2 * window(fold, 0.0, 0.1, 2.0, 6.0));
        screen.grade[1] = screen.grade[1].max(0.2 * window(fold, 0.5, 1.0, 3.0, 6.0));
    }
    let _ = sq;
}

/// How much of the frame the film's space covers (`TessPath.cover`).
pub fn cover(ct: f32) -> f32 {
    if (40.0..358.0).contains(&ct) { clamp01((ct - 40.0) / 12.0) } else { 0.0 }
}

/// The film's space scene and its screen (`TesseractFx.filmScreen`).
pub fn film(frame: &mut Frame, ct: f32, weight: f32, seed_f: f32) -> bool {
    let cover = cover(ct) * weight;
    let screen = &mut frame.screen;
    screen.synthetic = screen.synthetic.max(cover);
    if cover > 0.001 {
        screen.bloom = screen.bloom.max(0.6);
        frame.passes.extend(films::pass("tess_space", "tess_space", ct, Some("T0"), seed_f, Some(cover)));
    }
    let screen = &mut frame.screen;
    let w = window(ct, 62.0, 72.0, 108.0, 118.0);
    screen.zoom_blur = screen.zoom_blur.max(w * 0.25);
    let jump = window(ct, 118.0, 121.0, 124.0, 130.0);
    screen.aberration = screen.aberration.max(jump * 1.6);
    screen.shake = screen.shake.max(jump * 0.25);
    screen.add_flash(0xF0EAFF, jump * 0.15);
    screen.shake = screen.shake.max(window(ct, 150.0, 154.0, 160.0, 180.0) * 0.2);
    let pull = window(ct, 210.0, 230.0, 256.0, 262.0);
    screen.zoom_blur = screen.zoom_blur.max(pull * 0.18);
    screen.shake = screen.shake.max(pull * 0.15);
    let snap = window(ct, 248.0, 249.0, 250.0, 256.0);
    screen.aberration = screen.aberration.max(snap * 1.2);
    screen.add_flash(0xFFFFFF, snap * 0.3);
    let dive = window(ct, 262.0, 270.0, 288.0, 290.0);
    screen.zoom_blur = screen.zoom_blur.max(dive * 0.7);
    screen.shake = screen.shake.max(dive * 0.35);
    screen.aberration = screen.aberration.max(window(ct, 288.0, 290.0, 291.0, 296.0) * 0.8);
    if (336.0..358.0).contains(&ct) {
        let k = ((ct - 336.0) / 22.0).clamp(0.0, 1.0);
        screen.zoom_blur = screen.zoom_blur.max(k * k * 0.6);
    }
    screen.add_flash(0xFFFFFF, window(ct, 357.0, 358.0, 358.6, 362.0) * 1.2);
    cover >= 0.999
}

// ---- the film's camera ------------------------------------------------------------------------------------------

fn l(k: f32, a: f64, b: f64) -> f64 {
    a + (b - a) * f64::from(k)
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V, k: f64) -> V {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn up(a: V, y: f64) -> V {
    [a[0], a[1] + y, a[2]]
}
fn side(d: V) -> V {
    [-d[2], 0.0, d[0]]
}
fn home(x: &Subject, from: V) -> V {
    let d = [x.feet[0] - from[0], 0.0, x.feet[2] - from[2]];
    let n = (d[0] * d[0] + d[2] * d[2]).sqrt();
    if n * n < 1.0 { mul(x.forward, -1.0) } else { mul(d, 1.0 / n) }
}
/// Kept inside the drawn world, softly, and over the ground (`TessCamera.keep`).
fn keep(x: &Subject, g: &dyn Ground, pos: V) -> V {
    let reach = (g.reach() - 26.0).max(96.0);
    let soft = 0.8 * reach;
    let d = [pos[0] - x.feet[0], 0.0, pos[2] - x.feet[2]];
    let n = (d[0] * d[0] + d[2] * d[2]).sqrt();
    let kept = if n > soft {
        let k = soft + (reach - soft) * (1.0 - (-(n - soft) / (reach - soft)).exp());
        [x.feet[0] + d[0] / n * k, pos[1], x.feet[2] + d[2] / n * k]
    } else {
        pos
    };
    match g.top(kept[0], kept[2]) {
        Some(top) if kept[1] < top + 2.5 => [kept[0], top + 2.5, kept[2]],
        _ => kept,
    }
}
fn quad(a: V, b: V, c: V, t: f64) -> V {
    let s = 1.0 - t;
    add(add(mul(a, s * s), mul(b, 2.0 * s * t)), mul(c, t * t))
}
fn cube_root_ease(x: f32) -> f32 {
    if x < 0.5 { (x / 4.0).cbrt() } else { 1.0 - (2.0 * (1.0 - x)).cbrt() / 2.0 }
}

/// SS-05's film (`TesseractCutscenes`, 1.4.4).
pub fn cutscene(sq: Square) -> cs::Cutscene {
    let k = sq.scale.max(0.5);
    let at = |i: i32| 474 + i * 20;
    let mut shots: Vec<Shot> = Vec::new();
    shots.push(cs::Shot::cut(0, 8, |x, _, s| {
        let feet = x.feet;
        Pose { position: x.frame(feet, l(s, 1.35, 1.2), l(s, 1.7, 1.6), l(s, 2.1, 1.85)), look: x.frame(feet, 0.36, 1.38, 0.62), fov: 40.0, roll: 0.0 }
    }));
    let c = sq.centre();
    shots.push(cs::Shot::cut(8, 34, move |x, _, s| Pose {
        position: x.frame(x.feet, 2.0, l(s, 14.0, 26.0), -l(s, 10.0, 18.0)),
        look: up(c, l(s, 0.0, 30.0)),
        fov: l(s, 78.0, 84.0) as f32,
        roll: 0.0,
    }));
    shots.push(cs::Shot::sweep(34, 52, 3, move |x, g, s| {
        let h = home(x, c);
        let pos = keep(x, g, up(add(c, mul(h, 60.0 * (1.0 - f64::from(s)))), 40.0 + 420.0 * f64::from(s * s)));
        Pose { position: pos, look: add(c, mul(h, -20.0)), fov: l(s, 76.0, 84.0) as f32, roll: 0.0 }
    }));
    shots.push(cs::Shot::cut(52, 358, move |x, g, _| {
        let high = keep(x, g, up(c, 460.0));
        Pose { position: high, look: add(high, [0.0, 40.0, 10.0]), fov: 70.0, roll: 0.0 }
    }));
    shots.push(cs::Shot::sweep(358, 438, 2, move |x, g, s| {
        let h = home(x, c);
        let tt = 358.0 + f64::from(s) * 80.0;
        let angle = if tt < 398.0 { l(smooth(((tt - 358.0) / 30.0) as f32), 4.0, 56.0) } else { l(smooth(((tt - 398.0) / 34.0) as f32), 56.0, 20.0) };
        let open = f64::from(smooth(((tt - 398.0) / 30.0) as f32));
        let pos = keep(x, g, up(add(add(c, mul(h, (175.0 + 70.0 * open) * k)), mul(side(h), (20.0 + 30.0 * open) * k)), 6.0 + 30.0 * open));
        let a = angle.to_radians();
        let look = up(add(pos, mul(h, -a.cos() * 100.0)), a.sin() * 100.0);
        Pose { position: pos, look, fov: l(s, 84.0, 88.0) as f32, roll: 0.0 }
    }));
    shots.push(cs::Shot::cut(438, 462, move |x, g, s| {
        let h = home(x, c);
        let pos = keep(x, g, up(add(add(c, mul(h, l(s, 200.0, 180.0) * k)), mul(side(h), -50.0 * k)), l(s, 120.0, 105.0)));
        Pose { position: pos, look: up(c, 10.0), fov: 80.0, roll: 0.0 }
    }));
    let edge = 128.0 * sq.scale + 16.0;
    shots.push(cs::Shot::cut(462, at(0), move |x, g, s| {
        let h = home(x, c);
        let far = add(c, mul(h, edge));
        let pos = keep(x, g, up(add(far, mul(side(h), l(s, 150.0, 140.0))), 6.0));
        Pose { position: pos, look: up(add(far, mul(side(h), -60.0 * k)), l(s, 10.0, 30.0)), fov: 78.0, roll: l(s, 3.0, -2.0) as f32 }
    }));
    shots.push(cs::Shot::cut(at(0), at(1), move |x, g, s| {
        let h = home(x, c);
        let pos = keep(x, g, up(add(add(c, mul(h, l(s, 150.0, 140.0) * k)), mul(side(h), 24.0 * k)), 14.0));
        Pose { position: pos, look: up(c, l(s, 90.0, 140.0)), fov: 82.0, roll: l(s, -2.0, 2.0) as f32 }
    }));
    shots.push(cs::Shot::sweep(at(1), at(2), 2, move |x, g, s| {
        let h = home(x, c);
        let from = up(add(c, mul(h, 210.0 * k)), l(s, 70.0, 110.0));
        let a = l(s, -24.0, 12.0).to_radians();
        let (dx, dz) = (from[0] - c[0], from[2] - c[2]);
        let orbit = [c[0] + dx * a.cos() - dz * a.sin(), from[1], c[2] + dx * a.sin() + dz * a.cos()];
        Pose { position: keep(x, g, orbit), look: up(c, 90.0), fov: 80.0, roll: 0.0 }
    }));
    shots.push(cs::Shot::cut(at(2), at(3), move |x, g, s| {
        let h = home(x, c);
        let pit = add(add(c, mul(h, 40.0 * k)), mul(side(h), -18.0 * k));
        let floor = g.top(pit[0], pit[2]).unwrap_or(pit[1]);
        let pos = keep(x, g, [pit[0], floor + 2.5, pit[2]]);
        Pose { position: pos, look: up(add(pos, mul(h, -l(s, 30.0, 20.0))), 200.0), fov: 90.0, roll: l(s, 0.0, 14.0) as f32 }
    }));
    shots.push(cs::Shot::cut(at(3), 556, move |x, g, s| {
        let h = home(x, c);
        let pos = keep(x, g, up(add(add(c, mul(h, l(s, 230.0, 215.0) * k)), mul(side(h), 60.0 * k)), l(s, 150.0, 175.0)));
        Pose { position: pos, look: up(c, 80.0), fov: 86.0, roll: 0.0 }
    }));
    let engine = sq.engine(600.0);
    shots.push(cs::Shot::cut(556, 648, move |x, g, s| {
        let h = home(x, engine);
        let sd = side(h);
        let time = cube_root_ease(s) * 92.0;
        let near = smooth(time / 30.0);
        let far = cs::ease_in_out_pub(((time - 30.0) / 62.0).clamp(0.0, 1.0));
        let a = up(add(add(engine, mul(h, l(near, 70.0, 84.0) * k)), mul(sd, 26.0 * k)), l(near, -14.0, -6.0));
        let b = up(add(add(c, mul(h, 200.0 * k)), mul(sd, 70.0 * k)), 0.3 * (engine[1] - c[1]));
        let mid = [a[0] + (b[0] - a[0]) * 0.35, a[1] + (b[1] - a[1]) * 0.8, a[2] + (b[2] - a[2]) * 0.35];
        let pos = keep(x, g, quad(a, mid, b, f64::from(far)));
        let look = up(engine, -0.18 * (engine[1] - c[1]) * f64::from(far));
        Pose { position: pos, look, fov: l(far, l(near, 64.0, 74.0), 84.0) as f32, roll: l(far, l(near, -6.0, 3.0), 0.0) as f32 }
    }));
    shots.push(cs::Shot::sweep(648, 678, 10, move |x, g, s| {
        let h = mul(home(x, c), -1.0);
        let pos = up(add(add(x.feet, mul(h, -l(s, 3.2, 5.0))), mul(side(h), l(s, 1.4, 2.2))), l(s, 1.9, 2.8));
        Pose { position: keep(x, g, pos), look: engine, fov: l(s, 64.0, 72.0) as f32, roll: 0.0 }
    }));
    cs::Cutscene::from_shots(shots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cubes_lift_ring_by_ring_from_the_middle() {
        let t = cube_ticks();
        assert_eq!(t[3 * 8 + 3], 474);
        assert_eq!(t[3 * 8 + 4], 475);
        assert!(t[0] >= 534 && t[0] < 554, "a corner goes with the last ring: {}", t[0]);
        assert!(t.iter().all(|x| (474..560).contains(x)));
    }

    #[test]
    fn the_hypercube_has_sixteen_corners_round_its_middle() {
        let a = [0.3, 0.7, 1.1, 0.2];
        for i in 0..16 {
            let v = hyper(i, a);
            let r = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!(r > 0.5 && r < 6.0, "{i}: {v:?}");
        }
        // Unturned, the corners are the plain cube's, scaled by the projection.
        let v = hyper(0, [0.0; 4]);
        assert!((v[0] - v[1]).abs() < 1e-9 && (v[1] - v[2]).abs() < 1e-9);
    }

    #[test]
    fn cubes_rise_into_the_sky_above_the_square() {
        let start = sk::Start { skill: sk::Skill::Tesseract, mine: true, origin: [0.0, 70.0, -400.0], target: [0.0, 70.0, 0.0], yaw: 0.0, seed: 99, power: 0.4, speed: 1.0 };
        let sq = Square::new(&start, -64, 319);
        assert_eq!(sq.zone, 256);
        for i in 0..64 {
            let (base, height, side) = sq.cube(i % 8, i / 8);
            assert!(base > sq.ground && base + height <= 319, "{i}: {base} {height}");
            assert!(side > 0 && side <= sq.cell);
        }
        assert!(sq.engine_y(500.0) > sq.ground);
    }
}
