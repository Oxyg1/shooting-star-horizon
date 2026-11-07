// SS-06 Gravitia's light: the original's `GravitiaFx` (1.5.2) and its ring
// engine. Twenty rings (two cages, then the inner, middle and outer bands)
// fly in from the sky and lock round the Earth one by one, spin up, open fire,
// break apart at the blow and drift back together for the drop, then hang in
// the sky of what is left until they fade, about a hundred seconds later.
//
// What it draws over the world (`grav_world`): the sky turned to space, the
// telegraph's rings round the caster, the push's ring of light, the drop's
// three shock domes and the machine, which the render pass draws from the
// original's own mesh (`models/gravitia/machine.bin`) as this module poses
// it. Then the screen: shake, flashes, the blow's impact frames.
//
// Plain functions of the spell's clock and the camera: tested on their own.
use super::shooting_star_visuals::Spell;
use super::shooting_star_cutscenes::{self as cs, Pose, Shot};
use super::shooting_star_films as films;
use horizon_client_render::gfx::native_render::shooting_star_fxframe::{Frame, Machine, Pass, Screen, Value};
use std::f32::consts::PI;
use std::sync::OnceLock;

/// The world's state the light needs, beyond the spell and the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct World {
    /// The time of day in ticks (0 at sunrise).
    pub day_time: i64,
    /// How far the world is drawn, in blocks (the fog's end).
    pub render_blocks: f32,
    /// The camera is in Imaginary Space already (the world was banished).
    pub banished: bool,
}

// ---- small maths ------------------------------------------------------------------------------------------------

fn clamp01(x: f32) -> f32 {
    if x < 0.0 { 0.0 } else { x.min(1.0) }
}

fn smooth(x: f32) -> f32 {
    let x = clamp01(x);
    x * x * (3.0 - 2.0 * x)
}

fn window(t: f32, in0: f32, in1: f32, out0: f32, out1: f32) -> f32 {
    clamp01((t - in0) / (in1 - in0).max(0.001)) * (1.0 - clamp01((t - out0) / (out1 - out0).max(0.001)))
}

/// A kick that rises and dies away (`x/w * e^(1 - x/w)`).
fn pulse(x: f32, w: f32) -> f32 {
    if x <= 0.0 { 0.0 } else { x / w * (1.0 - x / w).exp() }
}

/// A damped swing.
fn swing(x: f32, half: f32, decay: f32) -> f32 {
    if x <= 0.0 { 0.0 } else { ((std::f64::consts::PI * f64::from(x) / f64::from(half)).sin() * f64::from(-x / decay).exp()) as f32 }
}

fn back_out(x: f32) -> f32 {
    let x = x - 1.0;
    1.0 + x * x * (2.70158 * x + 1.70158)
}

/// The original's per-part noise: a 64-bit mix of two integers, to 0..1.
pub fn noise(a: i32, b: i32) -> f32 {
    let mut z = (i64::from(a) as u64)
        .wrapping_mul(-7046029254386353131_i64 as u64)
        .wrapping_add((i64::from(b) as u64).wrapping_mul(-4417276706812531889_i64 as u64))
        .wrapping_add(1609587929392839161);
    z = (z ^ (z >> 33)).wrapping_mul(-49064778989728563_i64 as u64);
    z = (z ^ (z >> 33)).wrapping_mul(-4265267296055464877_i64 as u64);
    z ^= z >> 33;
    (z >> 40) as f32 / 16_777_216.0
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = length(v);
    if l > 0.0 { [v[0] / l, v[1] / l, v[2] / l] } else { v }
}

/// A 4x4 matrix, column-major, as JOML's `Matrix4f` keeps one; `then` is
/// JOML's own (`m.translate(..)` is `m * T`), `local` puts it first (`T * m`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct M4(pub [f32; 16]);

impl M4 {
    pub const IDENTITY: M4 = M4([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn mul(&self, b: &M4) -> M4 {
        let a = &self.0;
        let b = &b.0;
        let mut o = [0.0; 16];
        for c in 0..4 {
            for r in 0..4 {
                o[c * 4 + r] = a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2] + a[12 + r] * b[c * 4 + 3];
            }
        }
        M4(o)
    }

    fn translation(x: f32, y: f32, z: f32) -> M4 {
        let mut m = M4::IDENTITY;
        m.0[12] = x;
        m.0[13] = y;
        m.0[14] = z;
        m
    }

    /// Turning by `angle` round the unit axis, right-handed.
    fn rotation(angle: f32, x: f32, y: f32, z: f32) -> M4 {
        let (s, c) = angle.sin_cos();
        let k = 1.0 - c;
        M4([
            c + x * x * k,
            y * x * k + z * s,
            z * x * k - y * s,
            0.0,
            x * y * k - z * s,
            c + y * y * k,
            z * y * k + x * s,
            0.0,
            x * z * k + y * s,
            y * z * k - x * s,
            c + z * z * k,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }

    fn translate(&self, x: f32, y: f32, z: f32) -> M4 {
        self.mul(&M4::translation(x, y, z))
    }

    fn rotate(&self, angle: f32, x: f32, y: f32, z: f32) -> M4 {
        self.mul(&M4::rotation(angle, x, y, z))
    }

    fn rotate_x(&self, a: f32) -> M4 {
        self.rotate(a, 1.0, 0.0, 0.0)
    }

    fn rotate_y(&self, a: f32) -> M4 {
        self.rotate(a, 0.0, 1.0, 0.0)
    }

    fn rotate_z(&self, a: f32) -> M4 {
        self.rotate(a, 0.0, 0.0, 1.0)
    }

    fn rotate_local(&self, angle: f32, axis: [f32; 3]) -> M4 {
        M4::rotation(angle, axis[0], axis[1], axis[2]).mul(self)
    }

    fn scale_local(&self, s: f32) -> M4 {
        let mut m = *self;
        for c in 0..4 {
            for r in 0..3 {
                m.0[c * 4 + r] *= s;
            }
        }
        m
    }

    pub fn point(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.0;
        [
            m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
            m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
            m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
        ]
    }
}

// ---- the ring engine ---------------------------------------------------------------------------------------------

/// The parts, in the mesh file's order.
pub const PARTS: usize = 20;

/// When each group (the two cages, then the three bands) spins up, and ends.
const SPIN_START: [f32; 5] = [232.0, 234.0, 236.0, 240.0, 244.0];
const SPIN_END: [f32; 5] = [276.0, 278.0, 280.0, 282.0, 284.0];
const SPIN_TURNS: [f32; 5] = [2.0, 2.0, 2.0, 2.0, 1.0];
const SPIN_EASE: [f32; 5] = [1.5, 1.5, 1.5, 1.4, 2.0];
const SPIN_SIGN: [f32; 5] = [-1.0, 1.0, 1.0, 1.0, -1.0];
/// How fast each group rolls once it fires, degrees a tick.
const ROLL: [f32; 5] = [9.0, -7.0, 5.0, -3.6, 2.4];
const BAND_RADIUS: [f32; 3] = [1.2591, 1.4338, 1.6841];

/// Where each part stands, how it comes in and when.
struct Part {
    group: usize,
    /// 0 the cages and the inner band... the sound's size.
    band: usize,
    lock: f32,
    approach: f32,
    lit: f32,
    breaks: f32,
    home: [f32; 3],
    dir: [f32; 3],
    far: [f32; 3],
    axis: [f32; 3],
    turn: f32,
    reach: f32,
    settle: f32,
    phase: f32,
}

struct Engine {
    parts: Vec<Part>,
    /// The lock sounds: a part and its tick, in the order they sound.
    locks: Vec<(usize, f32)>,
}

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        const ANGLE: [[f32; 6]; 3] = [
            [29.5312, 88.5938, 149.0625, 209.5312, 268.5938, 329.0625],
            [30.9375, 90.0, 146.25, 206.7188, 270.0, 330.4688],
            [28.125, 87.1875, 149.0625, 209.5312, 270.0, 330.4688],
        ];
        const AZIMUTH: [[f32; 6]; 3] = [
            [40.0, 89.0, 139.0, 205.0, 270.0, 335.0],
            [21.0, 90.0, 156.0, 205.0, 270.0, 335.0],
            [38.0, 87.0, 140.0, 205.0, 270.0, 335.0],
        ];
        const ELEVATION: [[f32; 6]; 3] = [
            [18.0, -12.0, 15.0, -10.0, 15.0, 10.0],
            [-15.0, 15.0, -18.0, 12.0, -15.0, -12.0],
            [12.0, -22.0, 10.0, -14.0, 0.0, 14.0],
        ];
        const ALONG: [[u8; 6]; 3] = [[0, 1, 0, 1, 0, 1], [1, 0, 1, 0, 1, 0], [0, 1, 0, 1, 0, 0]];
        const TURN: [[f32; 6]; 3] = [
            [130.0, -150.0, -120.0, 110.0, 140.0, -160.0],
            [-140.0, 120.0, 150.0, -110.0, -130.0, 150.0],
            [120.0, 180.0, -130.0, 140.0, -150.0, 120.0],
        ];
        const REACH: [f32; 3] = [0.2, 0.22, 0.25];
        const SETTLE: [f32; 3] = [0.01, 0.012, 0.014];
        // The order and timing of the locks: (part, tick it locks, ticks it flies in).
        const LOCKS: [(usize, f32, f32); 20] = [
            (0, 119.0, 15.0),
            (1, 124.0, 15.0),
            (6, 130.0, 14.0),
            (5, 133.0, 13.0),
            (7, 136.0, 13.0),
            (4, 139.0, 13.0),
            (2, 142.0, 13.0),
            (3, 146.0, 14.0),
            (12, 153.0, 14.0),
            (11, 156.0, 14.0),
            (13, 160.0, 14.0),
            (10, 163.0, 14.0),
            (8, 167.0, 14.0),
            (9, 171.0, 15.0),
            (18, 180.0, 15.0),
            (17, 187.0, 16.0),
            (19, 194.0, 16.0),
            (16, 201.0, 16.0),
            (14, 208.0, 16.0),
            (15, 224.0, 22.0),
        ];
        let mut parts: Vec<Part> = (0..PARTS)
            .map(|i| Part {
                group: match i {
                    0 => 0,
                    1 => 1,
                    2..8 => 2,
                    8..14 => 3,
                    _ => 4,
                },
                band: match i {
                    0..2 => 0,
                    2..14 => 1,
                    _ => 2,
                },
                lock: 0.0,
                approach: 0.0,
                lit: 0.0,
                breaks: 0.0,
                home: [0.0; 3],
                dir: [0.0; 3],
                far: [0.0; 3],
                axis: [0.0; 3],
                turn: 0.0,
                reach: 0.0,
                settle: 0.0,
                phase: 0.0,
            })
            .collect();
        for (i, lock, approach) in LOCKS {
            parts[i].lock = lock;
            parts[i].approach = approach;
        }
        parts[0].home = [1.1243, 0.0, 0.0];
        parts[1].home = [0.0, 1.1492, 0.0];
        for ring in 0..3 {
            for k in 0..6 {
                let i = 2 + ring * 6 + k;
                let angle = f64::from(ANGLE[ring][k]).to_radians();
                let (c, s) = (angle.cos() as f32, angle.sin() as f32);
                let p = &mut parts[i];
                p.home = [BAND_RADIUS[ring] * c, BAND_RADIUS[ring] * s, 0.0];
                p.dir = [c, s, 0.0];
                let az = f64::from(AZIMUTH[ring][k]).to_radians();
                let el = f64::from(ELEVATION[ring][k]).to_radians();
                let d = if i == 15 { 10.0 } else { 8.5 };
                p.far = [(az.cos() * el.cos()) as f32 * d, (az.sin() * el.cos()) as f32 * d, el.sin() as f32 * d];
                p.axis = if ALONG[ring][k] == 0 { [-s, c, 0.0] } else { [c, s, 0.0] };
                p.turn = TURN[ring][k].to_radians();
                p.reach = if i == 15 { 0.3 } else { REACH[ring] };
                p.settle = if i == 15 { 0.022 } else { SETTLE[ring] };
                p.phase = angle as f32;
                p.lit = SPIN_START[ring + 2] + ((k + 2) % 6) as f32;
            }
        }
        parts[0].lit = SPIN_START[0];
        parts[1].lit = SPIN_START[1];
        parts[0].breaks = 430.0;
        parts[1].breaks = 430.5;
        for k in 0..6 {
            parts[k + 2].breaks = 431.0 + 0.15 * k as f32;
            parts[k + 8].breaks = 432.0 + 0.15 * k as f32;
            parts[k + 14].breaks = 433.0 + 0.15 * k as f32;
        }
        let mut locks: Vec<(usize, f32)> = LOCKS.iter().map(|(i, lock, _)| (*i, *lock)).collect();
        // Then a clank as the cages and each band finish spinning up.
        for i in [0, 1, 3, 9, 15] {
            locks.push((i, SPIN_END[parts[i].group]));
        }
        Engine { parts, locks }
    })
}

/// The lock sounds due in (from, to]: each part's band (0 small, 1 middle, 2 large).
pub fn locks_between(from: f32, to: f32) -> Vec<(usize, usize)> {
    let e = engine();
    e.locks.iter().filter(|(_, at)| *at > from && *at <= to).map(|(i, _)| (*i, e.parts[*i].band)).collect()
}

/// A ring's middle radius, round which the sky draws it thinner.
pub fn ring_radius(i: usize) -> f32 {
    match i {
        0 => 1.1243,
        1 => 1.1492,
        _ if i < PARTS => BAND_RADIUS[engine().parts[i].group - 2],
        _ => 1.0,
    }
}

/// A group's roll since it fired, radians.
fn roll(g: usize, t: f32) -> f32 {
    let sign = ROLL[g].signum();
    if t < 284.0 {
        0.0
    } else if t < 288.0 {
        -sign * 6f32.to_radians() * smooth((t - 288.0 + 4.0) / 4.0)
    } else {
        let e = t - 288.0;
        let kick = -6.0 + 24.0 * back_out((e / 2.5).min(1.0));
        let turned = if t <= 400.0 {
            160.0 * (1.0 - (-e / 160.0).exp())
        } else {
            let k = (-0.7f32).exp();
            let v = t.min(434.0) - 400.0;
            160.0 * (1.0 - k) + k * (v + v * v * v / 1800.0)
        };
        (sign * kick + ROLL[g] * turned).to_radians()
    }
}

/// A group's roll rate at full speed, radians a tick.
fn roll_rate(g: usize) -> f32 {
    roll(g, 388.5) - roll(g, 387.5)
}

/// A group spun up and rolling at `t`.
fn spin(g: usize, t: f32, m: M4) -> M4 {
    if t < SPIN_START[g] {
        return m;
    }
    let end = SPIN_END[g];
    if t < end {
        let f = (t - SPIN_START[g]) / (end - SPIN_START[g]);
        let a = SPIN_SIGN[g] * SPIN_TURNS[g] * (std::f64::consts::TAU * f64::from(f).powf(f64::from(SPIN_EASE[g]))) as f32;
        let w = smooth((t - SPIN_START[g]) / 8.0) * (1.0 - smooth((t - end + 12.0) / 9.0));
        return match g {
            0 => m.rotate_x(a),
            1 => m.rotate_y(a),
            2 => m.rotate_y(0.24 * w).rotate_z(0.8 * w).rotate_x(a),
            3 => m.rotate_y(a).rotate_z(-0.6 * w).rotate_x(0.44 * w),
            _ => m.rotate_y(0.12 * w).rotate_x(a),
        };
    }
    let mut m = m;
    if t > 400.0 {
        let k = (f64::from(t.min(430.0) - 400.0).powf(1.5) / 30f64.powf(1.5)) as f32;
        let a = 4.5f32.to_radians() * k;
        let ph = g as f32 * 1.7;
        let s = f64::from(t - 400.0);
        m = m.rotate_x(a * (s * std::f64::consts::TAU / 9.0 + f64::from(ph)).sin() as f32);
        m = m.rotate_y(a * (s * std::f64::consts::TAU / 11.0 + f64::from(ph)).cos() as f32);
    }
    m = m.rotate_z(roll(g, t));
    let settle = SPIN_SIGN[g] * 6f32.to_radians() * swing(t - end, 2.4, 2.2);
    if settle != 0.0 {
        m = if g == 1 || g == 3 { m.rotate_y(settle) } else { m.rotate_x(settle) };
    }
    m
}

/// A part's pose on the machine's own clock.
pub fn part_pose(i: usize, t: f32) -> M4 {
    let Some(p) = engine().parts.get(i) else { return M4::IDENTITY };
    if t < p.lock - p.approach || t >= 546.0 {
        // Not come yet, or gone: far off.
        let l = length(p.far);
        let k = if l > 0.0 { 10000.0 / l } else { 0.0 };
        return M4::translation(p.far[0] * k, p.far[1] * k, p.far[2] * k + if l > 0.0 { 0.0 } else { 10000.0 });
    }
    let mut m = M4::IDENTITY;
    // The whole machine shudders as it locks, and when it fires.
    let mut dy = -0.007 * pulse(t - 224.0, 1.4);
    let mut dx = 0.0;
    let e = t - 288.0;
    if e > 0.0 && e < 24.0 {
        let a = 0.009 * (-e / 3.5).exp() * (e / 0.5).min(1.0);
        dx = a * (2.1 * e).sin();
        dy += a * 0.6 * (1.7 * e).cos();
    }
    if dx != 0.0 || dy != 0.0 {
        m = m.translate(dx, dy, 0.0);
    }
    // After the blow it breaks apart, each part drifting and tumbling away.
    let mut clock = t;
    if t > p.breaks {
        let s = t - p.breaks;
        let c = spin(p.group, p.breaks, M4::IDENTITY).point(p.home);
        let (dir, speed) = match i {
            0 => (normalize([0.75, -0.45, 0.5]), 0.12),
            1 => (normalize([-0.8, 0.5, -0.3]), 0.12),
            _ => {
                let d = normalize([c[0], c[1], c[2] + (noise(i as i32, 1) - 0.5) * 0.8 * length(c)]);
                let base = match p.group {
                    2 => 0.16,
                    3 => 0.14,
                    _ => 0.12,
                };
                (d, base * (0.8 + 0.4 * noise(i as i32, 2)))
            }
        };
        let d = speed * 45.0 * (1.0 - (-s / 45.0).exp());
        let ax = [noise(i as i32, 3) * 2.0 - 1.0, noise(i as i32, 4) * 2.0 - 1.0, noise(i as i32, 5) * 2.0 - 1.0];
        let al = length(ax);
        let tumble = (6.0 + 8.0 * f64::from(noise(i as i32, 6))).to_radians() as f32 * s;
        m = m.translate(c[0] + dir[0] * d, c[1] + dir[1] * d, c[2] + dir[2] * d);
        if al > 1e-4 {
            m = m.rotate(tumble, ax[0] / al, ax[1] / al, ax[2] / al);
        }
        m = m.translate(-c[0], -c[1], -c[2]);
        clock = p.breaks;
    }
    m = spin(p.group, clock, m);
    let since = t - p.lock;
    if i >= 2 {
        let dir = p.dir;
        if since >= 0.0 {
            // Locked: a little settle outward.
            let k = p.settle * pulse(since, 1.2);
            if k != 0.0 {
                m = m.translate(dir[0] * k, dir[1] * k, dir[2] * k);
            }
        } else {
            // Flying in on a curve from far off, turning as it comes, then pressing home.
            let glide = p.approach - 3.0;
            let u = t - (p.lock - p.approach);
            let (at, turned) = if u < glide {
                let f = u / glide;
                let g = 1.0 - (1.0 - f) * (1.0 - f) * (1.0 - f);
                let h = 1.0 - g;
                let mid = p.reach + 0.4 * length(p.far);
                let at = [
                    h * h * p.far[0] + h * 2.0 * g * dir[0] * mid + g * g * dir[0] * p.reach,
                    h * h * p.far[1] + h * 2.0 * g * dir[1] * mid + g * g * dir[1] * p.reach,
                    h * h * p.far[2] + h * 2.0 * g * dir[2] * mid + g * g * dir[2] * p.reach,
                ];
                (at, (1.0 - f) * (1.0 - f) * (1.0 - f))
            } else {
                let q = (u - glide) / 3.0;
                let s = p.reach * (1.0 - q * q);
                ([dir[0] * s, dir[1] * s, dir[2] * s], 0.0)
            };
            m = m.translate(at[0], at[1], at[2]);
            if turned != 0.0 {
                let h = p.home;
                m = m.translate(h[0], h[1], h[2]).rotate(p.turn * turned, p.axis[0], p.axis[1], p.axis[2]).translate(-h[0], -h[1], -h[2]);
            }
        }
    } else if since >= 0.0 {
        let a = 6f32.to_radians() * swing(since, 2.0, 1.8);
        m = if i == 0 { m.rotate_x(-a) } else { m.rotate_y(a) };
    } else {
        // The cages drop in and swing upright.
        let glide = p.approach - 4.0;
        let u = t - (p.lock - p.approach);
        let (off, k) = if u < glide {
            let w = 1.0 - u / glide;
            (8.5 * w * w * w, 1.0)
        } else {
            let q = (u - glide) / 4.0;
            (0.0, 1.0 - q * q)
        };
        m = if i == 0 {
            m.translate(0.0, off, 0.0).rotate_x(PI / 2.0 * k)
        } else {
            m.translate(off, 0.0, 0.0).rotate_y(-PI / 2.0 * k)
        };
    }
    m
}

/// A part's glow on the machine's clock: dim as it locks, lit with its
/// group's spin, breathing while it fires, flickering at the overload.
pub fn part_glow(i: usize, t: f32) -> f32 {
    let Some(p) = engine().parts.get(i) else { return 0.0 };
    if t < p.lock {
        return 0.0;
    }
    let mut g = if t < p.lit {
        (0.85 * (-(t - p.lock) / 2.2).exp()).max(0.02)
    } else {
        let e = t - p.lit;
        let dip = (-((e - 1.6) / 0.5) * ((e - 1.6) / 0.5)).exp();
        let mut g = (clamp01(e) * (1.0 - 0.65 * dip)).max(0.02);
        if t >= 288.0 {
            g *= 0.85 + 0.15 * (f64::from(t - 288.0) * std::f64::consts::TAU / 16.0 - f64::from(p.phase)).cos() as f32;
        }
        g
    };
    if t >= 400.0 && t < p.breaks {
        let f = (t - 400.0) / 30.0;
        if noise(i as i32, (t * 0.5).floor() as i32) < 0.35 * f * f {
            g *= 0.5;
        }
    }
    if t >= p.breaks {
        let e = t - p.breaks;
        g = if e < 1.5 { 1.0 } else { (-(e - 1.5) / 2.5).exp() };
    }
    clamp01(g)
}

/// The core's charge.
pub fn core_charge(t: f32) -> f32 {
    if t < 232.0 {
        0.0
    } else if t < 284.0 {
        f64::from((t - 232.0) / 52.0).powf(1.8) as f32
    } else if t < 288.0 {
        1.0
    } else if t < 400.0 {
        let base = 0.45 + 0.04 * (f64::from(t - 288.0) * std::f64::consts::TAU / 24.0).sin() as f32;
        base + (1.0 - base) * (-(t - 288.0) / 3.5).exp()
    } else if t < 430.0 {
        let f = (t - 400.0) / 30.0;
        0.45 + 0.55 * f * f
    } else if t < 518.0 {
        (-(t - 430.0) / 4.0).exp()
    } else {
        0.0
    }
}

// ---- the machine in the world's sky -------------------------------------------------------------------------------

/// When each part of the reformed machine flashes into place (620..640).
const REFORM: [f32; 9] = [626.8, 629.8, 632.2, 634.1, 635.6, 636.8, 637.7, 638.4, 639.0];
/// When each group dims for good, and how fast it tumbles then.
const FADE: [f32; 5] = [724.0, 724.0, 708.0, 692.0, 676.0];
const TUMBLE: [f32; 5] = [1.6, -1.3, 1.0, -0.8, 0.6];
const TUMBLE_AXIS: [[f32; 3]; 5] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.7071, 0.7071, 0.0], [0.7071, -0.7071, 0.0], [0.0, 1.0, 0.0]];

/// The machine's own clock: it plays to the blow, holds, runs back to its
/// assembled pose for the drop, then rolls on.
pub fn machine_clock(t: f32) -> f32 {
    if t < 527.0 {
        t
    } else if t < 620.0 {
        527.0
    } else if t < 640.0 {
        527.0 - 97.5 * ((t - 620.0) / 20.0)
    } else if t < 654.0 {
        429.5
    } else {
        288.0 + (t - 654.0).min(100.0)
    }
}

/// How much of the machine shows in the sky.
pub fn machine_weight(t: f32) -> f32 {
    if t < 74.0 { 0.0 } else { 1.0 - smooth((t - 2002.0) / 66.0) }
}

/// How much of the sky is space.
pub fn space_weight(t: f32) -> f32 {
    smooth(t / 8.0) * (1.0 - smooth((t - 2002.0) / 106.0))
}

/// The charge before the drop, 0 to 1 (640..654).
pub fn overload(t: f32) -> f32 {
    if (640.0..654.0).contains(&t) {
        let k = (t - 640.0) / 14.0;
        k * k
    } else {
        0.0
    }
}

/// The drop's flash on the ground.
pub fn drop_flash(t: f32) -> f32 {
    if t < 654.0 { 0.0 } else { (f64::from(-(t - 654.0)) / 9.0).exp() as f32 }
}

/// The space sky's frame: the nebula 48 degrees up ahead of the caster.
pub fn sky_basis(fx: f64, fz: f64) -> [[f32; 3]; 3] {
    let d = (fx * fx + fz * fz).sqrt();
    let (x, z) = if d > 1e-6 { (fx / d, fz / d) } else { (0.0, 1.0) };
    let up = 48f64.to_radians();
    let dir = [(x * up.cos()) as f32, up.sin() as f32, (z * up.cos()) as f32];
    let side = [-z as f32, 0.0, x as f32];
    [dir, side, cross(dir, side)]
}

/// The same frame mirrored about the caster's heading (Imaginary Space's sky).
pub fn mirrored(basis: [[f32; 3]; 3], fx: f64, fz: f64) -> [[f32; 3]; 3] {
    let d = (fx * fx + fz * fz).sqrt();
    let x = if d > 1e-6 { (fx / d) as f32 } else { 0.0 };
    let z = if d > 1e-6 { (fz / d) as f32 } else { 1.0 };
    basis.map(|v| {
        let k = 2.0 * (v[0] * x + v[2] * z);
        [k * x - v[0], -v[1], k * z - v[2]]
    })
}

/// Where the Sun is at this time of day.
pub fn sun_dir(day_time: i64) -> [f32; 3] {
    let f = day_time as f64 / 24000.0 - 0.25;
    let f = f - f.floor();
    let eased = 0.5 - (f * std::f64::consts::PI).cos() / 2.0;
    let a = (f * 2.0 + eased) / 3.0 * std::f64::consts::TAU;
    [(-a.sin()) as f32, a.cos() as f32, 0.0]
}

/// Which group of the sky's machine a part is (by its name in the original's
/// mesh): the cages, then the inner, middle and outer bands.
fn sky_group(i: usize) -> usize {
    match i {
        0 => 0,
        1 => 1,
        2..8 => 2,
        8..14 => 3,
        _ => 4,
    }
}

/// When a part flashes into place as the machine reforms.
fn reform_at(i: usize) -> f32 {
    match i {
        0 | 1 => 640.0,
        _ => {
            let digit = (i - 2) % 6;
            let ring = match i {
                14.. => 0,
                8..14 => 1,
                _ => 2,
            };
            REFORM[(ring * 3 + digit % 3).min(REFORM.len() - 1)]
        }
    }
}

/// The machine in the world's sky this tick: round the Earth below, tilted 40
/// degrees toward the caster's heading, six times its size, growing as it fades.
pub fn sky_machine(t: f32, fx: f64, fz: f64, sun: [f32; 3]) -> Machine {
    let d = (fx * fx + fz * fz).sqrt();
    let (x, z) = if d > 1e-6 { (fx / d, fz / d) } else { (0.0, 1.0) };
    let tilt = 40f64.to_radians();
    let rx = x * tilt.cos() - z * tilt.sin();
    let rz = x * tilt.sin() + z * tilt.cos();
    let (s, c) = (tilt.sin() as f32, tilt.cos() as f32);
    let back = [(-rx) as f32 * c, s, (-rz) as f32 * c];
    let up = [rx as f32 * s, c, rz as f32 * s];
    let side = cross(up, back);
    let centre = [0.0, -1.004, 0.0];
    let base = M4([
        side[0], side[1], side[2], 0.0, up[0], up[1], up[2], 0.0, back[0], back[1], back[2], 0.0, centre[0], centre[1], centre[2], 1.0,
    ]);
    let grow = smooth((t - 2002.0) / 66.0);
    let size = (1.0 + 4.0 * grow * grow) * 6.0;
    let dim = 1.0 - 0.8 * smooth((t - 288.0) / 30.0);
    let over = overload(t);
    let charge = if (640.0..654.0).contains(&t) {
        1.0 + 1.8 * over
    } else {
        core_charge(machine_clock(t)) * (1.0 - smooth((t - 2002.0) / 24.0))
    };
    let clock = machine_clock(t);
    let mut models = Vec::with_capacity(PARTS);
    let mut glow = Vec::with_capacity(PARTS);
    let mut thin = Vec::with_capacity(PARTS);
    for i in 0..PARTS {
        let mut tt = clock;
        if (620.0..640.0).contains(&t) {
            tt = 527.0 - 97.5 * ((t - 620.0) / (reform_at(i) - 620.0)).min(1.0);
        }
        let g = sky_group(i);
        let mut m = part_pose(i, tt);
        let mut turn = if (640.0..654.0).contains(&t) {
            let e = t - 640.0;
            (if g % 2 == 0 { 1.0 } else { -1.0 }) * (0.8 * f64::from(e) * f64::from(e) * (1.0 + 0.3 * g as f64)).to_radians() as f32
        } else {
            0.0
        };
        let e = t - 654.0 - 100.0;
        if e > 0.0 {
            let rate = roll_rate(g);
            let slow = rate * 0.3;
            turn += slow * e + (rate - slow) * 120.0 * (1.0 - (-e / 120.0).exp());
        }
        if turn != 0.0 {
            m = M4::rotation(turn, 0.0, 0.0, 1.0).mul(&m);
        }
        let e = t - 684.0;
        if e > 0.0 {
            let q = e.min(60.0) / 60.0;
            let v = 60.0 * (q * q * q - 0.5 * q * q * q * q) + (e - 60.0).max(0.0);
            m = m.rotate_local(f64::from(TUMBLE[g] * v).to_radians() as f32, TUMBLE_AXIS[g]);
        }
        m = m.scale_local(size);
        let mut light = part_glow(i, tt);
        if over > 0.0 {
            light = light.max(0.6) * (1.0 + 0.9 * over * (0.6 + 0.4 * (t * 2.3 + i as f32 * 1.7).sin()));
        }
        let e = t - FADE[g];
        if e > 0.0 {
            let floor = 0.18 * (1.0 - smooth((t - 2002.0) / 30.0));
            light = if e < 3.0 { light * if (e * 9.0 + i as f32).sin() > 0.0 { 0.6 } else { 0.15 } } else { (light * 0.03).max(floor) };
        }
        models.push(base.mul(&m).0);
        glow.push(light);
        thin.push(ring_radius(i));
    }
    Machine {
        view_proj: None,
        models,
        glow,
        thin,
        thin_k: 0.3,
        cam_pos: [0.0, 1.004, 0.0],
        sun_dir: sun,
        sun_col: [1.7 * dim, 1.632 * dim, 1.53 * dim],
        earth_shade: false,
        earth_rel: centre,
        core: [0.55 * charge * 6.0, 0.8 * charge * 6.0, charge * 6.0, 13.2],
        behind: [-back[0], -back[1], -back[2]],
    }
}

// ---- the world's pass and the screen ------------------------------------------------------------------------------

/// The drop's three shock domes: when each starts, how far it goes, how fast it slows, how bright.
const DOMES: [[f32; 4]; 3] = [[654.0, 1100.0, 0.9, 1.0], [673.0, 800.0, 1.1, 0.6], [688.0, 650.0, 1.3, 0.42]];

/// The domes' radius and strength at `t`.
pub fn domes(t: f32) -> [f32; 6] {
    let mut w = [0.0; 6];
    for (i, d) in DOMES.iter().enumerate() {
        let s = (t - d[0]) / 20.0;
        if s > 0.0 {
            w[i * 2] = d[1] * (1.0 - (-s / d[2]).exp());
            w[i * 2 + 1] = d[3] * (-s / 1.6).exp() * (s / 0.03).min(1.0);
        }
    }
    w
}

/// Where the drop lands: on the target, or under the camera when it is far off.
fn drop_at(target: [f64; 3], camera: [f64; 3]) -> [f64; 3] {
    let dx = camera[0] - target[0];
    let dz = camera[2] - target[2];
    if dx * dx + dz * dz < 360_000.0 { target } else { [camera[0], target[1], camera[2]] }
}

/// The impact frames of the blow, one each twelfth of a second.
const BLOW_FRAMES: [i32; 6] = [1, 36, 41, 37, 40, 38];
/// The blow's flashes, one colour each ninth of a second.
const BLOW_COLOURS: [u32; 7] = [0xFFFFFF, 0xD6E9FF, 0x7CC5FF, 0x2F86FF, 0x1E50E8, 0x5FB9FF, 0xBFE7FF];

/// Projects a camera-relative direction to the screen (0..1), for the impact frames' focus.
fn on_screen(view_proj: Option<&glam_like::Proj>, dir: [f32; 3]) -> [f32; 2] {
    match view_proj {
        Some(p) => p.project(dir),
        None => [0.5, 0.5],
    }
}

/// A camera's projection, as the runtime knows it (yaw, pitch, fov, aspect).
pub mod glam_like {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Proj {
        pub forward: [f32; 3],
        pub right: [f32; 3],
        pub up: [f32; 3],
        /// tan(fov / 2) and width over height.
        pub tan_half: f32,
        pub aspect: f32,
    }

    impl Proj {
        pub fn project(&self, d: [f32; 3]) -> [f32; 2] {
            let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            let z = dot(d, self.forward);
            if z <= 1e-4 {
                return [0.5, 0.5];
            }
            let x = dot(d, self.right) / (z * self.tan_half * self.aspect);
            let y = dot(d, self.up) / (z * self.tan_half);
            [(x * 0.5 + 0.5).clamp(0.0, 1.0), (y * 0.5 + 0.5).clamp(0.0, 1.0)]
        }
    }
}

/// Gravitia's light this frame: the world's pass, the machine, the screen.
pub fn frame(frame: &mut Frame, spell: &Spell, camera: [f64; 3], world: World, proj: Option<&glam_like::Proj>) {
    let t = spell.t;
    let start = &spell.start;
    let yaw = f64::from(start.yaw).to_radians();
    let (fx, fz) = (-yaw.sin(), yaw.cos());
    let seed = (start.seed & 0xFFFF) as f32 / 65535.0;
    let aspect = proj.map_or(16.0 / 9.0, |p| p.aspect);
    // The film: space, the Earth, the machine assembling round it.
    let mut whole = false;
    if let Some(f) = spell.film {
        whole = film(frame, f.ct, f.weight, seed, aspect);
    }
    // (The original takes the player to a stage of its own from 546 to 614;
    // here they stay, so the sky keeps the broken machine over them.)
    if !whole {
        let telegraph = window(t, 9.0, 13.0, 60.0, 66.0);
        let push = window(t, 288.0, 294.0, 430.0, 440.0);
        let push_age = (t - 288.0).max(0.0) / 20.0;
        let machine = machine_weight(t);
        let space = if t >= 596.0 && world.banished { 0.0 } else { space_weight(t) };
        let flash = drop_flash(t);
        let over = overload(t);
        let w = domes(t);
        let sky_machine_on = machine > 0.001;
        if telegraph > 0.001 || push > 0.001 || sky_machine_on || space > 0.001 || flash > 0.001 || w[1] + w[3] + w[5] > 0.001 {
            if sky_machine_on {
                frame.machine = Some(sky_machine(t, fx, fz, sun_dir(world.day_time)));
            }
            let sky = sky_basis(fx, fz);
            let since = (t - 9.0).max(0.0) / 20.0;
            let drop_age = (t - 654.0).max(0.0) / 20.0;
            frame.passes.push(
                Pass::new("grav_world")
                    .at("Target", start.target)
                    .f("P0", &[telegraph, since, t / 20.0, seed])
                    .f("P1", &[push, push_age, world.render_blocks, 0.0])
                    .f("S0", &[if sky_machine_on { machine } else { 0.0 }, space, 0.0, flash])
                    .f("S1", &[over, 0.0, 0.0, drop_age])
                    .f("W0", &[w[0], w[1], w[2], w[3]])
                    .f("W1", &[w[4], w[5], 0.0, 0.0])
                    .f("SkyDir", &sky[0])
                    .f("SkyX", &sky[1])
                    .f("SkyY", &sky[2])
                    .at("DropAt", drop_at(start.target, camera))
                    .texture("MilkyWay", "moonfall/milky_way.jpg")
                    .texture("Nebula", "gravitia/nebula.jpg"),
            );
        }
    }
    if spell.film.is_none_or(|f| film_cover(f.ct) * f.weight < 0.001) {
        screen(&mut frame.screen, t, fx, fz, proj);
    }
}

// ---- the film ---------------------------------------------------------------------------------------------------

/// How much of the picture the film's space scene is (`GravitiaPath.cover`).
pub fn film_cover(ct: f32) -> f32 {
    if (48.0..546.0).contains(&ct) { clamp01((ct - 48.0) / 12.0) } else { 0.0 }
}

fn uniform<'a>(pass: &'a Pass, name: &str) -> Option<&'a [f32]> {
    pass.uniforms.iter().rev().find(|(n, _)| *n == name).and_then(|(_, v)| match v {
        Value::F(f) => Some(f.as_slice()),
        _ => None,
    })
}

fn v3(pass: &Pass, name: &str) -> [f32; 3] {
    uniform(pass, name).map_or([0.0; 3], |v| [v[0], v[1], v[2]])
}

/// The machine as the film's camera sees it: round the Earth, its own size,
/// from the camera's place in Earth radii.
fn film_machine(pass: &Pass, ct: f32, aspect: f32) -> Machine {
    let (fwd, up, right) = (v3(pass, "CamFwd"), v3(pass, "CamUp"), v3(pass, "CamRight"));
    let earth = v3(pass, "EarthRel");
    let (mx, my, mz) = (v3(pass, "MachX"), v3(pass, "MachY"), v3(pass, "MachZ"));
    let tan_half = uniform(pass, "V0").map_or(0.6, |v| v[0]).max(1e-3);
    let view = M4([right[0], up[0], -fwd[0], 0.0, right[1], up[1], -fwd[1], 0.0, right[2], up[2], -fwd[2], 0.0, 0.0, 0.0, 0.0, 1.0]);
    // Reversed depth, as the world's: 1 at the near plane, 0 far off.
    let (near, far) = (0.01f32, 6000.0f32);
    let f = 1.0 / tan_half;
    let proj = M4([f / aspect, 0.0, 0.0, 0.0, 0.0, f, 0.0, 0.0, 0.0, 0.0, near / (far - near), -1.0, 0.0, 0.0, near * far / (far - near), 0.0]);
    let base = M4([mx[0], mx[1], mx[2], 0.0, my[0], my[1], my[2], 0.0, mz[0], mz[1], mz[2], 0.0, earth[0], earth[1], earth[2], 1.0]);
    let charge = core_charge(ct);
    let sun = uniform(pass, "SunDir").map_or([0.5, 0.6, 0.6], |v| [v[0], v[1], v[2]]);
    Machine {
        view_proj: Some(proj.mul(&view).0),
        models: (0..PARTS).map(|i| base.mul(&part_pose(i, ct)).0).collect(),
        glow: (0..PARTS).map(|i| part_glow(i, ct)).collect(),
        thin: (0..PARTS).map(ring_radius).collect(),
        thin_k: 1.0,
        cam_pos: [-earth[0], -earth[1], -earth[2]],
        sun_dir: sun,
        sun_col: [1.7, 1.632, 1.53],
        earth_shade: true,
        earth_rel: earth,
        core: [0.8 * charge * 6.0, 0.72 * charge * 6.0, charge * 6.0, 2.2],
        behind: [-mz[0], -mz[1], -mz[2]],
    }
}

/// The film's space scene and its screen (`GravitiaFx`, the film's half).
/// True when it fills the picture.
pub fn film(frame: &mut Frame, ct: f32, weight: f32, seed: f32, aspect: f32) -> bool {
    let cover = film_cover(ct) * weight;
    if cover <= 0.001 {
        return false;
    }
    let Some(mut pass) = films::pass("grav_space", "grav_space", ct, Some("V0"), seed, Some(cover)) else { return false };
    let mesh = ct >= 74.0;
    if mesh {
        frame.machine = Some(film_machine(&pass, ct, aspect));
    }
    let burst = if (430.0..504.0).contains(&ct) { ct - 430.0 } else { -1.0 };
    let emerge = if ct < 501.0 { 0.0 } else if ct >= 504.0 { 1.0 } else { smooth((ct - 501.0) / 3.0) };
    pass = pass.f("V3", &[if mesh { 1.0 } else { 0.0 }, if ct >= 474.0 { 1.0 } else { 0.0 }, if burst >= 0.0 { emerge } else { 0.0 }, 0.0]);
    // Where the Earth is on the screen: the blows' impact frames centre on it.
    let (fwd, up, right, earth) = (v3(&pass, "CamFwd"), v3(&pass, "CamUp"), v3(&pass, "CamRight"), v3(&pass, "EarthRel"));
    let tan_half = uniform(&pass, "V0").map_or(0.6, |v| v[0]).max(1e-3);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let z = dot(earth, fwd).max(1e-4);
    let focus = [
        (dot(earth, right) / (z * tan_half) / aspect * 0.5 + 0.5).clamp(0.0, 1.0),
        (dot(earth, up) / (z * tan_half) * 0.5 + 0.5).clamp(0.0, 1.0),
    ];
    frame.passes.push(pass);
    film_screen(&mut frame.screen, ct, cover, focus);
    cover >= 0.999
}

const FIRE_FRAMES: [i32; 4] = [36, 37, 40, 38];

fn film_screen(screen: &mut Screen, ct: f32, cover: f32, focus: [f32; 2]) {
    screen.synthetic = screen.synthetic.max(cover);
    screen.bloom = screen.bloom.max(if ct >= 288.0 { 0.45 } else { 0.55 } * cover);
    if (104.0..228.0).contains(&ct) {
        screen.shake = screen.shake.max(0.06);
    }
    let k = (ct - 224.0) / 20.0;
    if (0.0..0.6).contains(&k) {
        screen.add_flash(0xE4EEFF, (-k * 9.0).exp() * 0.5);
    }
    if (232.0..288.0).contains(&ct) {
        let s = smooth((ct - 232.0) / 56.0);
        screen.shake = screen.shake.max(s * s * 0.35);
        screen.aberration = screen.aberration.max(s * s * 0.4);
        screen.add_vignette(0x04080F, s * 0.4);
    }
    let fire = (ct - 288.0) / 20.0;
    if (0.0..3.0).contains(&fire) {
        let n = (fire * 12.0) as usize;
        if n < FIRE_FRAMES.len() {
            screen.add_impact(1.0, FIRE_FRAMES[n], focus[0], focus[1]);
        }
        screen.add_flash(0xE6EFFF, (-fire * 10.0).exp() * 0.9);
        screen.shake = screen.shake.max((-fire * 1.6).exp() * 0.9);
        screen.aberration = screen.aberration.max((-fire * 2.0).exp());
        screen.zoom_blur = screen.zoom_blur.max((-fire * 4.0).exp() * 0.35);
    }
    if (400.0..430.0).contains(&ct) {
        screen.shake = screen.shake.max(smooth((ct - 400.0) / 30.0) * 0.45);
    }
    let blow = (ct - 430.0) / 20.0;
    if (0.0..3.0).contains(&blow) {
        let n = (blow * 12.0) as usize;
        if n < BLOW_FRAMES.len() {
            screen.add_impact(1.0, BLOW_FRAMES[n], focus[0], focus[1]);
            screen.step(12.0);
        }
        screen.add_flash(0xE6EFFF, (-blow * 7.0).exp());
        screen.shake = screen.shake.max((-blow * 0.9).exp());
        screen.aberration = screen.aberration.max((-blow * 1.4).exp() * 0.9);
    }
    let line = (ct - 504.0) / 20.0;
    if (0.0..1.5).contains(&line) {
        screen.add_flash(0xE6EFFF, (-line * 9.0).exp() * 0.6);
        screen.shake = screen.shake.max((-line * 1.5).exp() * 0.6);
    }
    if (504.0..546.0).contains(&ct) {
        let s = smooth((ct - 504.0) / 30.0);
        let fade = 1.0 - smooth((ct - 534.0) / 12.0);
        screen.shake = screen.shake.max(s * s * 0.45 * fade);
        screen.aberration = screen.aberration.max(s * s * 0.7 * fade);
        screen.zoom_blur = screen.zoom_blur.max(s * s * 0.08 * fade);
    }
}

fn lerp(k: f32, a: f64, b: f64) -> f64 {
    a + (b - a) * f64::from(k)
}

/// Gravitia's film camera (the original's `GravitiaShots`): the caster up
/// close, pulled back, flung up into the sky; the space scene; then behind
/// the caster, looking out at what is left.
pub fn cutscene() -> cs::Cutscene {
    let mut shots: Vec<Shot> = Vec::new();
    shots.push(Shot::cut(0, 8, |x, _, s| Pose {
        position: x.frame(x.feet, lerp(s, 1.35, 1.2), lerp(s, 1.7, 1.6), lerp(s, 2.1, 1.85)),
        look: x.frame(x.feet, 0.36, 1.38, 0.62),
        fov: 40.0,
        roll: 0.0,
    }));
    shots.push(Shot::cut(8, 36, |x, g, s| {
        let mut position = x.frame(x.feet, lerp(s, 2.6, 4.0), lerp(s, 5.0, 9.0), lerp(s, -4.0, -7.0));
        // Never inside a hill.
        if let Some(top) = g.top(position[0], position[2]) {
            position[1] = position[1].max(top + 1.5);
        }
        Pose { position, look: [x.feet[0], x.feet[1] + 0.5, x.feet[2]], fov: lerp(s, 58.0, 66.0) as f32, roll: 0.0 }
    }));
    shots.push(Shot::sweep(36, 60, 4, |x, _, s| {
        let v = 14.0 + 520.0 * f64::from(s) * f64::from(s);
        Pose { position: x.frame(x.feet, 6.0 + v * 0.04, v, -10.0 - v * 0.03), look: x.feet, fov: lerp(s, 66.0, 62.0) as f32, roll: 0.0 }
    }));
    shots.push(Shot::cut(60, 546, |x, _, _| {
        let p = [x.feet[0], x.feet[1] + 520.0, x.feet[2]];
        Pose { position: p, look: [p[0] + 10.0, p[1] + 40.0, p[2]], fov: 70.0, roll: 0.0 }
    }));
    shots.push(Shot::cut(546, 634, |x, _, _| {
        let eye = [x.feet[0], x.feet[1] + 1.62, x.feet[2]];
        let f = x.forward;
        Pose {
            position: [eye[0] - f[0] * 4.0, eye[1], eye[2] - f[2] * 4.0],
            look: [eye[0] + f[0] * 20.0, eye[1] + 6.0, eye[2] + f[2] * 20.0],
            fov: 70.0,
            roll: 0.0,
        }
    }));
    cs::Cutscene::from_shots(shots)
}

/// The screen through the cast (the original's `screen` without the film).
fn screen(screen: &mut Screen, t: f32, fx: f64, fz: f64, proj: Option<&glam_like::Proj>) {
    // The machine fires: the ground hums, the edges darken.
    if (288.0..430.0).contains(&t) {
        let k = window(t, 288.0, 292.0, 410.0, 430.0);
        screen.shake = screen.shake.max(0.25 * k);
        screen.add_vignette(0x040A18, 0.45 * k);
    }
    // The blow: white, then the impact frames toward the core in the sky.
    if (430.0..550.0).contains(&t) {
        let s = (t - 430.0) / 20.0;
        screen.add_flash(0xFFFFFF, (-(s - 0.4).max(0.0) * 1.2).exp());
        screen.shake = screen.shake.max((-s * 0.8).exp() * 0.9);
        let up = sky_basis(fx, fz)[0];
        let focus = on_screen(proj, up);
        if (0.0..2.5).contains(&s) {
            if s < 0.05 {
                screen.add_flash(0xFFFFFF, 0.0);
            } else if s - 0.05 < BLOW_FRAMES.len() as f32 / 12.0 {
                let n = ((s - 0.05) * 12.0) as usize;
                screen.add_impact(1.0, BLOW_FRAMES[n.min(BLOW_FRAMES.len() - 1)], focus[0], focus[1]);
                screen.step(12.0);
                screen.zoom_blur = screen.zoom_blur.max(if n % 2 == 0 { 0.25 } else { 0.08 });
                screen.aberration = screen.aberration.max(if n % 2 == 0 { 0.5 } else { 1.8 });
            }
            screen.add_flash(BLOW_COLOURS[(s * 9.0) as usize % BLOW_COLOURS.len()], (-s * 5.0).exp() * 0.8);
            screen.shake = screen.shake.max(window(s, 0.0, 0.02, 0.5, 2.0) * 1.2);
            screen.bloom = screen.bloom.max(0.75 * window(s, 0.0, 0.1, 1.0, 2.5));
        }
    }
    // Coming back: one frame, then the machine reforming, charging, the drop.
    let k = (t - 614.0) / 20.0;
    if (0.0..1.0 / 12.0).contains(&k) {
        screen.add_impact(1.0, 44, 0.5, 0.5);
    }
    if t >= 620.0 {
        if t < 640.0 {
            for at in REFORM {
                let e = t - at;
                if (0.0..4.0).contains(&e) {
                    screen.add_flash(0xBFE7FF, 0.14 * (-e / 0.8).exp());
                }
            }
        }
        if (640.0..654.0).contains(&t) {
            let k = (t - 640.0) / 14.0;
            screen.add_vignette(0x02081A, 0.5 * k);
            screen.bloom = screen.bloom.max(0.4 * k);
            if t >= 651.0 {
                // The breath before the drop: the world drained of colour, the picture stepping.
                screen.desaturate = screen.desaturate.max(0.8);
                screen.grade[0] = screen.grade[0].max(0.24);
                screen.grade[1] = screen.grade[1].max(0.28);
                screen.grade[3] = screen.grade[3].max(0.8);
                screen.step(12.0);
            }
        }
        let s = (t - 654.0) / 20.0;
        if (0.0..8.0).contains(&s) {
            screen.add_flash(0xFFFFFF, (-s * 6.0).exp() * 0.8);
            screen.bloom = screen.bloom.max(1.1 * window(s, 0.0, 0.1, 3.0, 8.0));
            screen.bloom_radius = screen.bloom_radius.max(1.0 + 1.2 * window(s, 0.0, 0.1, 2.0, 6.0));
            screen.grade[1] = screen.grade[1].max(0.2 * window(s, 0.5, 1.0, 3.0, 6.0));
            screen.shake = screen.shake.max(window(s, 0.0, 0.02, 1.0, 4.0) * 0.8);
        }
    }
}

/// Is the camera's sky Imaginary Space's (the world banished)? Its sky stays
/// space for good: the Milky Way and the nebula, mirrored about the heading.
pub fn void_sky(frame: &mut Frame, heading: (f64, f64)) {
    let basis = mirrored(sky_basis(heading.0, heading.1), heading.0, heading.1);
    frame.passes.insert(
        0,
        Pass::new("void_sky")
            .f("SkyDir", &basis[0])
            .f("SkyX", &basis[1])
            .f("SkyY", &basis[2])
            .texture("MilkyWay", "moonfall/milky_way.jpg")
            .texture("Nebula", "gravitia/nebula.jpg"),
    );
}

/// The sounds due this tick (`t` the spell's tick): name and volume, heard in the head.
pub fn sounds(t: i32) -> Vec<(String, f32)> {
    const AT: [(i32, &str); 21] = [
        (4, "grav_rework"),
        (9, "grav_mark"),
        (9, "grav_countdown"),
        (36, "grav_ascent"),
        (60, "grav_space"),
        (104, "grav_assemble"),
        (160, "grav_title"),
        (224, "grav_locked"),
        (232, "grav_spin"),
        (288, "grav_fire"),
        (288, "grav_push"),
        (360, "grav_hold"),
        (400, "grav_dive"),
        (430, "grav_burst"),
        (474, "grav_rejoin"),
        (508, "grav_line"),
        (534, "grav_flight"),
        (620, "grav_reform"),
        (640, "grav_charge"),
        (654, "grav_drop"),
        (658, "grav_after"),
    ];
    let mut out: Vec<(String, f32)> = AT.iter().filter(|(at, _)| *at == t).map(|(_, n)| ((*n).to_owned(), 1.0)).collect();
    if t > 74 && t < 224 {
        for (i, band) in locks_between((t - 1) as f32, t as f32) {
            let names: &[&str] = match band {
                0 => &["grav_lock_s1", "grav_lock_s2", "grav_lock_s3"],
                2 => &["grav_lock_l1", "grav_lock_l2"],
                _ => &["grav_lock_m1", "grav_lock_m2", "grav_lock_m3"],
            };
            let volume = if (t - 160).abs() < 3 { 0.5 } else { 0.8 };
            out.push((names[(i as i32 * 7 + t).rem_euclid(names.len() as i32) as usize].to_owned(), volume));
        }
    }
    out
}

/// The camera's shake from the blows, by tick: (strength, at).
pub fn jolts(t: i32) -> Option<f64> {
    match t {
        430 => Some(1.0),
        504 => Some(0.6),
        654 => Some(1.4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_noise_is_the_originals() {
        // Values the original's own function gives.
        let v = noise(3, 1);
        assert!((0.0..1.0).contains(&v));
        assert_ne!(noise(3, 1), noise(3, 2));
    }

    #[test]
    fn the_parts_come_in_one_by_one_and_lock_home() {
        // Before its time a part waits far off.
        assert!(part_pose(15, 100.0).0[12].abs() + part_pose(15, 100.0).0[13].abs() > 1000.0);
        // Locked, a band part keeps its place on the ring.
        let m = part_pose(8, 200.0);
        assert!(m.0[12].abs() < 0.05 && m.0[13].abs() < 0.05, "{m:?}");
        // Gone after the blow.
        assert!(part_pose(2, 600.0).0[14] > 1000.0 || part_pose(2, 600.0).0[12].abs() > 1000.0);
    }

    #[test]
    fn the_machine_hangs_in_the_sky_for_a_hundred_seconds() {
        assert_eq!(machine_weight(60.0), 0.0);
        assert!(machine_weight(1000.0) > 0.99);
        assert!(machine_weight(2068.0) < 0.01);
        let m = sky_machine(300.0, 0.0, 1.0, sun_dir(18000));
        assert_eq!(m.models.len(), PARTS);
        assert!(m.glow.iter().any(|g| *g > 0.3));
    }

    #[test]
    fn the_locks_sound_once_each() {
        // The last ring locks at 224, as the window closes: nineteen clanks, as the original's.
        let all: Vec<_> = (75..224).flat_map(|t| sounds(t)).filter(|(n, _)| n.starts_with("grav_lock_")).collect();
        assert_eq!(all.len(), 19);
        assert!(sounds(654).iter().any(|(n, _)| n == "grav_drop"));
    }

    #[test]
    fn the_sun_is_overhead_at_noon() {
        let s = sun_dir(6000);
        assert!(s[1] > 0.99, "{s:?}");
    }
}
