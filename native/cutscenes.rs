// The films' cameras: the original's `Cutscene`, its `Shot`s and the
// `CutsceneDirector`'s blend with the player's own view. Each spell's film
// is its list of shots, each a path of the camera over its share of the
// film's time; the shots are the original's (`StarCutscenes`,
// `GungnirCutscenes`, `SevenStarsCutscenes`), point for point.
//
// The world comes in through `Ground`, so this is plain math, tested alone.
use super::shooting_star_skills::{self as sk, Skill};

pub type V = [f64; 3];

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V, k: f64) -> V {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn up(a: V, y: f64) -> V {
    [a[0], a[1] + y, a[2]]
}
fn len(a: V) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn lerp3(a: V, b: V, k: f64) -> V {
    add(a, mul(sub(b, a), k))
}
fn l(k: f32, a: f64, b: f64) -> f64 {
    a + (b - a) * f64::from(k)
}

/// Where the camera stands and looks, its field of view and roll (degrees).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: V,
    pub look: V,
    pub fov: f32,
    pub roll: f32,
}

impl Pose {
    pub fn yaw(&self) -> f32 {
        let d = sub(self.look, self.position);
        (-d[0]).atan2(d[2]).to_degrees() as f32
    }
    pub fn pitch(&self) -> f32 {
        let d = sub(self.look, self.position);
        (-(d[1].atan2(d[0].hypot(d[2]))).to_degrees()) as f32
    }
    pub fn lerp(&self, o: &Pose, k: f32) -> Pose {
        let kk = f64::from(k);
        Pose {
            position: lerp3(self.position, o.position, kk),
            look: lerp3(self.look, o.look, kk),
            fov: self.fov + (o.fov - self.fov) * k,
            roll: self.roll + (o.roll - self.roll) * k,
        }
    }
}

/// The world as the cameras need it: the ground under a column and whether
/// a block stops the eye.
pub trait Ground {
    /// The top of the motion-blocking ground at `x, z` (one above the
    /// highest block), if the column is loaded and has any.
    fn top(&self, x: f64, z: f64) -> Option<f64>;
    fn solid(&self, at: V) -> bool;
    /// How far the world is drawn round the player (`getEffectiveRenderDistance() * 16`).
    fn reach(&self) -> f64 {
        192.0
    }
}

/// The caster as the film sees them (`Subject`).
#[derive(Clone, Copy, Debug)]
pub struct Subject {
    pub feet: V,
    pub target: V,
    /// Where they faced as they cast, flat.
    pub forward: V,
}

impl Subject {
    fn right(&self) -> V {
        [-self.forward[2], 0.0, self.forward[0]]
    }
    fn frame(&self, anchor: V, r: f64, u: f64, f: f64) -> V {
        let (fw, rt) = (self.forward, self.right());
        add(anchor, [rt[0] * r + fw[0] * f, u, rt[2] * r + fw[2] * f])
    }
    /// The flat way from `from` back to the caster.
    fn home_from(&self, from: V) -> V {
        let d = sub(self.feet, from);
        let flat = [d[0], 0.0, d[2]];
        if flat[0] * flat[0] + flat[2] * flat[2] < 1.0 { mul(self.forward, -1.0) } else { mul(flat, 1.0 / len(flat)) }
    }
    fn home(&self) -> V {
        self.home_from(self.target)
    }
}

fn side(d: V) -> V {
    [-d[2], 0.0, d[0]]
}

fn orbit(point: V, pivot: V, degrees: f64) -> V {
    let a = degrees.to_radians();
    let (x, z) = (point[0] - pivot[0], point[2] - pivot[2]);
    [pivot[0] + x * a.cos() - z * a.sin(), point[1], pivot[2] + x * a.sin() + z * a.cos()]
}

/// `Terrain.above`: no lower than `margin` over the ground.
fn above(g: &dyn Ground, pos: V, margin: f64) -> V {
    match g.top(pos[0], pos[2]) {
        Some(top) if pos[1] < top + margin => [pos[0], top + margin, pos[2]],
        _ => pos,
    }
}

/// `Terrain.sighted`: raised until nothing stands between it and `look`.
fn sighted(g: &dyn Ground, pos: V, look: V, reach: f64) -> V {
    let mut lift = 0.0;
    while lift <= reach {
        let at = up(pos, lift);
        match hit(g, at, look) {
            None => return at,
            Some(h) if len(sub(h, look)) < 6.0 => return at,
            _ => {}
        }
        lift += 6.0;
    }
    pos
}

/// The first solid block from `from` to `to`, walked half a block at a time.
fn hit(g: &dyn Ground, from: V, to: V) -> Option<V> {
    let d = sub(to, from);
    let n = len(d);
    if n < 1e-3 {
        return None;
    }
    let steps = ((n * 2.0).ceil() as usize).min(1024);
    for i in 1..=steps {
        let at = lerp3(from, to, i as f64 / steps as f64);
        if g.solid(at) {
            return Some(at);
        }
    }
    None
}

/// `CutsceneDirector.clip`: pulled in front of the wall between the camera
/// and what it looks at.
fn clip(g: &dyn Ground, pose: Pose) -> Pose {
    let mut from = up(pose.look, 0.6);
    let toward = sub(pose.position, from);
    let length = len(toward);
    if length < 0.001 {
        return pose;
    }
    let step = mul(toward, 1.0 / length);
    let mut walked = 0.0;
    while g.solid(from) {
        if walked >= length {
            return pose;
        }
        from = add(from, step);
        walked += 1.0;
    }
    match hit(g, from, pose.position) {
        None => pose,
        Some(h) => {
            let back = sub(pose.position, from);
            let dir = mul(back, 1.0 / len(back).max(1e-6));
            // Half a step short of the block, then the original's 0.45.
            Pose { position: sub(h, mul(dir, 0.7)), ..pose }
        }
    }
}

pub type Path = Box<dyn Fn(&Subject, &dyn Ground, f32) -> Pose + Send + Sync>;

/// One shot: a path of the camera from `start` to `end` (film ticks),
/// eased in and out, crossfaded over `blend` ticks from the shot before.
pub struct Shot {
    pub start: i32,
    pub end: i32,
    pub blend: i32,
    pub path: Path,
}

fn cut(start: i32, end: i32, path: impl Fn(&Subject, &dyn Ground, f32) -> Pose + Send + Sync + 'static) -> Shot {
    Shot { start, end, blend: 0, path: Box::new(path) }
}

fn sweep(start: i32, end: i32, blend: i32, path: impl Fn(&Subject, &dyn Ground, f32) -> Pose + Send + Sync + 'static) -> Shot {
    Shot { start, end, blend, path: Box::new(path) }
}

pub fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn ease_in_out(x: f32) -> f32 {
    if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
}

const FADE_OUT: f32 = 14.0;

/// A spell's film: its shots, over the spell's own clock.
pub struct Cutscene {
    pub shots: Vec<Shot>,
    pub end: f32,
}

impl Cutscene {
    fn new(shots: Vec<Shot>) -> Cutscene {
        let end = shots.last().map_or(0, |s| s.end) as f32 + FADE_OUT;
        Cutscene { shots, end }
    }

    /// How much the film holds the camera at film tick `t` (cut in hard,
    /// as the spells' films are), `skipped` ticks after a skip.
    pub fn weight(&self, t: f32, skipped: Option<f32>) -> f32 {
        if t < 0.0 {
            return 0.0;
        }
        let mut out = 1.0 - smooth((t - (self.end - FADE_OUT)) / FADE_OUT);
        if let Some(s) = skipped {
            out = out.min(1.0 - smooth(s / 8.0));
        }
        out.max(0.0)
    }

    pub fn finished(&self, t: f32) -> bool {
        t > self.end
    }

    fn evaluate(&self, shot: &Shot, t: f32, subject: &Subject, g: &dyn Ground) -> Pose {
        let k = ((t - shot.start as f32) / ((shot.end - shot.start) as f32).max(1.0)).clamp(0.0, 1.0);
        (shot.path)(subject, g, ease_in_out(k))
    }

    /// The film's camera at tick `t`.
    pub fn pose(&self, t: f32, subject: &Subject, g: &dyn Ground) -> Option<Pose> {
        let mut index = 0;
        for (i, shot) in self.shots.iter().enumerate() {
            if t >= shot.start as f32 {
                index = i;
            }
        }
        let current = self.shots.get(index)?;
        let mut pose = self.evaluate(current, t, subject, g);
        if index > 0 && current.blend > 0 && t - (current.start as f32) < current.blend as f32 {
            let previous = &self.shots[index - 1];
            let from = self.evaluate(previous, previous.end as f32, subject, g);
            pose = from.lerp(&pose, smooth((t - current.start as f32) / current.blend as f32));
        }
        Some(clip(g, pose))
    }
}

/// The vanilla camera blended toward the film's by `weight`
/// (`CutsceneDirector.apply`).
pub fn blend(position: V, yaw: f32, pitch: f32, shot: &Pose, weight: f32) -> Pose {
    let (y, p) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    let look = add(position, [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]);
    let vanilla = Pose { position, look, fov: shot.fov, roll: 0.0 };
    vanilla.lerp(shot, weight)
}

/// The first shot of every film: the hand on the remote.
fn hand() -> Shot {
    cut(0, 8, |x, _, k| {
        let feet = x.feet;
        let hand = x.frame(feet, 0.36, 1.38, 0.62);
        Pose { position: x.frame(feet, l(k, 1.35, 1.2), l(k, 1.7, 1.6), l(k, 2.1, 1.85)), look: hand, fov: 40.0, roll: 0.0 }
    })
}

/// SS-01's film (`StarCutscenes.theShootingStar`).
pub fn star(r: f64) -> Cutscene {
    let n = (r / 200.0).clamp(0.3, 1.0);
    let climb_end = 72;
    let impact = 352;
    let shots = vec![
        hand(),
        cut(8, 30, |x, _, k| Pose {
            position: x.frame(x.feet, 1.1, 2.1, -3.2),
            look: up(x.target, l(k, 30.0, 4.0)),
            fov: l(k, 58.0, 30.0) as f32,
            roll: 0.0,
        }),
        cut(30, 46, |x, _, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, l(k, 7.5, 6.0))), mul(side(h), 2.8)), l(k, 1.0, 1.4));
            Pose { position: pos, look: up(t, l(k * k, 0.3, 24.0)), fov: l(k, 64.0, 70.0) as f32, roll: l(k, 0.0, 4.0) as f32 }
        }),
        sweep(46, climb_end, 3, |x, _, k| {
            let (t, h) = (x.target, x.home());
            let rise = 6.0 + 470.0 * f64::from(k * k);
            Pose { position: up(add(t, mul(h, 4.0 + rise * 0.04)), rise), look: t, fov: l(k, 70.0, 64.0) as f32, roll: 0.0 }
        }),
        cut(climb_end, 342, |x, _, _| {
            let high = up(x.target, 480.0);
            Pose { position: high, look: up(add(high, mul(x.home(), 10.0)), 40.0), fov: 70.0, roll: 0.0 }
        }),
        cut(342, impact, |x, _, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, l(k, 150.0, 135.0))), mul(side(h), 22.0)), 2.5);
            Pose { position: pos, look: up(t, l(k.sqrt(), 330.0, 30.0)), fov: l(k, 62.0, 56.0) as f32, roll: l(k, 2.0, -2.0) as f32 }
        }),
        cut(impact, impact + 22, move |x, _, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, r + l(k, 150.0, 140.0) * n)), mul(side(h), 40.0 * n)), l(k, 150.0, 138.0) * n);
            Pose { position: pos, look: up(t, l(k, 70.0, 90.0) * n), fov: 84.0, roll: l(k, -3.0, 0.0) as f32 }
        }),
        sweep(impact + 22, impact + 70, 6, move |x, _, k| {
            let (t, h) = (x.target, x.home());
            let from = up(add(t, mul(h, r + 175.0 * n)), l(k, 95.0, 175.0) * n);
            Pose { position: orbit(from, t, l(k, -18.0, 16.0)), look: up(t, l(k, 130.0, 230.0) * n), fov: l(k, 76.0, 80.0) as f32, roll: 0.0 }
        }),
        cut(impact + 70, 462, move |x, _, k| {
            let (t, h) = (x.target, x.home());
            let edge = add(t, mul(h, r + 16.0 * n));
            let pos = up(add(edge, mul(side(h), l(k, -24.0, 24.0) * n)), 4.0);
            let look = up(add(add(t, mul(h, r - 30.0 * n)), mul(side(h), l(k, 30.0, -30.0) * n)), l(k, 30.0, 90.0) * n);
            Pose { position: pos, look, fov: 74.0, roll: l(k, 6.0, -6.0) as f32 }
        }),
        cut(462, 494, move |x, _, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, r + l(k, 90.0, 70.0) * n)), mul(side(h), 30.0 * n)), l(k, 300.0, 270.0) * n);
            Pose { position: pos, look: up(t, l(k, 40.0, 0.0) * n), fov: 72.0, roll: l(k, 0.0, 6.0) as f32 }
        }),
        sweep(494, 530, 8, move |x, _, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, r + l(k, 60.0, 30.0) * n)), mul(side(h), l(k, 20.0, -10.0) * n)), l(k, 150.0, 60.0) * n);
            let look = up(add(t, mul(h, l(k, 40.0, 0.0) * n)), l(k, -40.0, -110.0) * n);
            Pose { position: pos, look, fov: l(k, 78.0, 72.0) as f32, roll: 0.0 }
        }),
        sweep(530, 564, 10, |x, _, k| back_to_caster(x, x.target, k, -70.0)),
    ];
    Cutscene::new(shots)
}

/// The last shot of the films: behind the caster, the strike before them.
fn back_to_caster(x: &Subject, toward: V, k: f32, look_up: f64) -> Pose {
    let h = x.home_from(toward);
    let pos = up(add(add(x.feet, mul(h, l(k, 3.2, 5.0))), mul(side(h), l(k, 1.4, 2.2))), l(k, 1.9, 2.8));
    Pose { position: pos, look: up(toward, look_up), fov: l(k, 64.0, 72.0) as f32, roll: 0.0 }
}

/// SS-03's film (`GungnirCutscenes.gungnir`).
pub fn gungnir(power: f32) -> Cutscene {
    let space = 62;
    let back = 300;
    let impact = 322;
    let boom = impact + 19;
    let big = sk::gungnir_scale(power).max(0.3);
    let ring_wait = sk::gungnir_front(f64::from(impact + 19), sk::gungnir_radius(power));
    let shots = vec![
        hand(),
        cut(8, 28, |x, _, k| Pose {
            position: x.frame(x.feet, 1.1, 2.2, -3.3),
            look: up(x.target, l(k, 24.0, 3.0)),
            fov: l(k, 60.0, 34.0) as f32,
            roll: 0.0,
        }),
        cut(28, 44, |x, g, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, l(k, 8.0, 6.5))), mul(side(h), 3.0)), l(k, 1.0, 1.3));
            Pose { position: above(g, pos, 2.5), look: up(t, l(k * k, 2.0, 160.0)), fov: l(k, 62.0, 72.0) as f32, roll: l(k, 0.0, -3.0) as f32 }
        }),
        sweep(44, space + 4, 3, |x, _, k| {
            let (t, h) = (x.target, x.home());
            let rise = 6.0 + 480.0 * f64::from(k * k);
            Pose { position: up(add(t, mul(h, 4.0 + rise * 0.04)), rise), look: t, fov: l(k, 72.0, 64.0) as f32, roll: 0.0 }
        }),
        cut(space + 4, back, |x, _, _| {
            let high = up(x.target, 480.0);
            Pose { position: high, look: up(add(high, mul(x.home(), 10.0)), 40.0), fov: 70.0, roll: 0.0 }
        }),
        cut(back, impact, move |x, g, k| {
            let (t, h) = (x.target, x.home());
            let e = k * k;
            let fall = 20.0 + 9000.0 * (1.0 - f64::from(k)).max(0.0).powf(0.75);
            let pos = up(add(add(t, mul(h, l(k, 160.0, 150.0) * big)), mul(side(h), 20.0 * big)), 2.5);
            Pose { position: above(g, pos, 2.5), look: up(t, fall.min(520.0) * l(e, 0.95, 0.8)), fov: l(k, 64.0, 58.0) as f32, roll: l(k, 2.0, -1.0) as f32 }
        }),
        cut(impact, impact + 7, move |x, g, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, ring_wait)), mul(side(h), -50.0 * big)), l(k, 390.0, 370.0) * big);
            Pose { position: above(g, pos, 2.5), look: up(add(t, mul(h, 50.0 * big)), 10.0), fov: l(k, 74.0, 78.0) as f32, roll: l(k, -4.0, -6.0) as f32 }
        }),
        cut(impact + 7, impact + 13, move |x, g, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, ring_wait + 18.0 * big)), mul(side(h), 46.0 * big)), 3.0);
            Pose { position: above(g, pos, 2.5), look: up(t, l(k, 240.0, 300.0)), fov: l(k, 76.0, 80.0) as f32, roll: l(k, 5.0, 3.0) as f32 }
        }),
        cut(impact + 13, boom, move |x, g, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, ring_wait)), mul(side(h), 12.0 * big)), 5.0);
            Pose { position: above(g, pos, 2.5), look: up(t, l(k, 30.0, 20.0)), fov: l(k, 66.0, 72.0) as f32, roll: 0.0 }
        }),
        sweep(boom, boom + 40, 2, move |x, g, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, ring_wait - 4.0 * f64::from(k) * big)), mul(side(h), l(k, 12.0, 20.0) * big)), l(k * k, 5.0, 48.0) * big);
            Pose { position: above(g, pos, 2.5), look: up(t, l(k, 20.0, 70.0)), fov: l(k, 72.0, 66.0) as f32, roll: l(k, -2.0, 0.0) as f32 }
        }),
        sweep(boom + 40, boom + 100, 8, move |x, g, k| {
            let (t, h) = (x.target, x.home());
            let pos = up(add(add(t, mul(h, l(k, 200.0, 105.0) * big)), mul(side(h), l(k, -40.0, -22.0) * big)), l(k, 14.0, 34.0));
            Pose { position: above(g, pos, 2.5), look: up(t, l(k * k, 40.0, 300.0)), fov: l(k, 62.0, 76.0) as f32, roll: 0.0 }
        }),
        sweep(boom + 100, boom + 134, 10, |x, g, k| {
            let p = back_to_caster(x, x.target, k, 90.0);
            Pose { position: above(g, p.position, 2.5), ..p }
        }),
    ];
    Cutscene::new(shots)
}

/// SS-04's film (`SevenStarsCutscenes.sevenStars`); `nodes` are where the
/// seven land.
pub fn seven(power: f32, nodes: [V; 7]) -> Cutscene {
    let big = f64::from(power);
    let sky = 900.0 * big;
    let dissolve = 58;
    let back = 272;
    let centre = mul(nodes.iter().fold([0.0; 3], |a, n| add(a, *n)), 1.0 / 7.0);
    // Kept inside the drawn world (`near`).
    fn near(x: &Subject, g: &dyn Ground, pos: V) -> V {
        let d = [pos[0] - x.feet[0], 0.0, pos[2] - x.feet[2]];
        let n = len(d);
        let reach = (g.reach() - 26.0).max(96.0);
        let kept = if n <= reach { pos } else { [x.feet[0] + d[0] / n * reach, pos[1], x.feet[2] + d[2] / n * reach] };
        above(g, kept, 2.5)
    }
    fn seen(g: &dyn Ground, pos: V, look: V, fov: f64, roll: f64) -> Pose {
        Pose { position: sighted(g, pos, look, 160.0), look, fov: fov as f32, roll: roll as f32 }
    }
    let mut shots = vec![
        hand(),
        cut(8, 24, move |x, _, k| Pose {
            position: x.frame(x.feet, 4.0, l(k, 28.0, 44.0), -l(k, 20.0, 28.0)),
            look: up(x.target, -12.0),
            fov: l(k, 74.0, 80.0) as f32,
            roll: 0.0,
        }),
        sweep(24, dissolve - 6, 4, move |x, _, k| Pose {
            position: x.frame(x.feet, 1.2, 1.9, -1.6),
            look: up(centre, sky * l(k, 0.6, 1.0)),
            fov: l(k, 84.0, 76.0) as f32,
            roll: l(k, -4.0, 6.0) as f32,
        }),
        sweep(dissolve - 6, dissolve + 12, 3, move |x, g, k| {
            let pos = near(x, g, up(centre, 20.0 + 420.0 * f64::from(k * k)));
            Pose { position: pos, look: add(pos, [0.0, 100.0, 12.0]), fov: 76.0, roll: 0.0 }
        }),
        cut(dissolve + 12, back, move |x, g, _| {
            let high = near(x, g, up(centre, 460.0));
            Pose { position: high, look: add(high, [0.0, 40.0, 10.0]), fov: 70.0, roll: 0.0 }
        }),
    ];
    for (i, node) in nodes.iter().copied().enumerate() {
        let land = sk::dipper_land_tick(i);
        let fall_from = if i == 0 { back } else { sk::dipper_land_tick(i - 1) + 6 };
        let r = sk::dipper_crater_radius(i, big);
        shots.push(cut(fall_from, land, move |x, g, k| {
            let h = x.home_from(node);
            let pos = near(x, g, up(add(add(node, mul(h, r + 48.0)), mul(side(h), 40.0)), 3.0));
            let rise = sky * (1.0 - f64::from(k)).max(0.0).powf(0.75);
            seen(g, pos, up(node, (rise * 0.85).max(14.0 * big)), l(k, 72.0, 66.0), l(k, 2.0, -2.0))
        }));
        let until = if i < 6 { land + 6 } else { land + 10 };
        shots.push(cut(land, until, move |x, g, k| {
            let h = x.home_from(node);
            let pos = near(x, g, up(add(add(node, mul(h, r * 1.25 + 30.0)), mul(side(h), -60.0)), 30.0 + r * 0.35));
            seen(g, pos, up(node, 6.0 * big), l(k, 76.0, 80.0), l(k, -3.0, 0.0))
        }));
    }
    let link_from = sk::dipper_land_tick(6) + 10;
    shots.push(sweep(link_from, 440, 4, move |x, g, k| {
        let h = x.home_from(centre);
        let from = up(add(centre, mul(h, l(k, 260.0, 220.0))), l(k, 150.0, 180.0));
        seen(g, near(x, g, orbit(from, centre, l(k, -14.0, 14.0))), up(centre, -10.0 * big), 78.0, 0.0)
    }));
    shots.push(cut(440, 476, move |x, g, k| {
        let h = x.home_from(centre);
        let pos = near(x, g, up(add(add(centre, mul(h, l(k, 300.0, 260.0))), mul(side(h), 80.0)), l(k, 110.0, 140.0)));
        seen(g, pos, up(centre, l(k, 150.0, 260.0) * big), 88.0, l(k, -3.0, 3.0))
    }));
    let four = nodes[4];
    let r4 = sk::dipper_crater_radius(4, big);
    shots.push(sweep(476, 526, 8, move |x, g, k| {
        let h = x.home_from(four);
        let pos = near(x, g, up(add(add(four, mul(h, r4 * l(k, 0.8, 0.6))), mul(side(h), l(k, 30.0, 50.0))), l(k, 24.0, 48.0)));
        seen(g, pos, up(four, l(k * k, 10.0, 220.0) * big), l(k, 70.0, 80.0), 0.0)
    }));
    shots.push(sweep(526, 556, 10, move |x, _, k| {
        // The original turns `home` round here, so the camera stands on the caster's other side.
        let h = mul(x.home_from(centre), -1.0);
        let pos = up(add(add(x.feet, mul(h, -l(k, 3.2, 5.0))), mul(side(h), l(k, 1.4, 2.2))), l(k, 1.9, 2.8));
        Pose { position: pos, look: up(centre, 60.0 * big), fov: l(k, 64.0, 72.0) as f32, roll: 0.0 }
    }));
    Cutscene::new(shots)
}

/// The film of a spell, if it has one yet.
pub fn of(start: &sk::Start, nodes: Option<[V; 7]>) -> Option<Cutscene> {
    match start.skill {
        Skill::ShootingStar => Some(star(f64::from(start.power))),
        Skill::Gungnir => Some(gungnir(start.power)),
        Skill::SevenStars => nodes.map(|n| seven(start.power, n)),
        Skill::Tesseract | Skill::Gravitia => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat(f64);
    impl Ground for Flat {
        fn top(&self, _: f64, _: f64) -> Option<f64> {
            Some(self.0)
        }
        fn solid(&self, at: V) -> bool {
            at[1] < self.0
        }
    }

    fn subject() -> Subject {
        Subject { feet: [0.0, 64.0, -300.0], target: [0.0, 64.0, 0.0], forward: [0.0, 0.0, 1.0] }
    }

    #[test]
    fn the_railgun_film_climbs_to_orbit_and_comes_back() {
        let film = star(200.0);
        let g = Flat(64.0);
        let s = subject();
        let first = film.pose(2.0, &s, &g).expect("pose");
        assert!(len(sub(first.position, s.feet)) < 4.0, "starts at the caster's hand");
        let orbit = film.pose(200.0, &s, &g).expect("pose");
        assert!((orbit.position[1] - 544.0).abs() < 1e-6, "480 over the target: {:?}", orbit.position);
        let last = film.pose(560.0, &s, &g).expect("pose");
        assert!(len(sub(last.position, s.feet)) < 8.0);
        assert_eq!(film.weight(100.0, None), 1.0);
        assert_eq!(film.weight(film.end + 1.0, None), 0.0);
        assert!(film.weight(100.0, Some(10.0)) < 0.01, "a skip fades the film out");
    }

    #[test]
    fn no_shot_puts_the_camera_inside_the_ground() {
        let g = Flat(64.0);
        let s = subject();
        let nodes = [[0.0, 64.0, 0.0]; 7].map(|n| n);
        for film in [star(48.0), gungnir(1.0), seven(1.0, nodes)] {
            let mut t = 0.0;
            while t < film.end {
                let p = film.pose(t, &s, &g).expect("pose");
                assert!(p.position[1] >= 64.0 - 0.75, "t={t}: {:?}", p.position);
                assert!(p.fov > 20.0 && p.fov < 100.0);
                t += 0.5;
            }
        }
    }

    #[test]
    fn the_camera_angles_point_at_the_look() {
        let p = Pose { position: [0.0, 0.0, 0.0], look: [0.0, 0.0, 10.0], fov: 70.0, roll: 0.0 };
        assert!(p.yaw().abs() < 1e-4 && p.pitch().abs() < 1e-4);
        let down = Pose { look: [0.0, -10.0, 0.0001], ..p };
        assert!((down.pitch() - 90.0).abs() < 0.01);
        let b = blend([0.0, 0.0, 0.0], 0.0, 0.0, &p, 0.0);
        assert!((b.look[2] - 1.0).abs() < 1e-9, "weight 0 keeps the player's view");
    }
}
