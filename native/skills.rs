// The Stellar Remote's skills: the original 1.5.2's five, as it defines them, and the port's own: their
// names, colours, timelines, recharges and tuning ranges, the shapes both
// halves need (the beam's width, Gungnir's shock front, the Dipper's stars,
// the Tesseract's grid) and the messages the two halves trade.
//
// The server's spells live in `shooting_star_spells`, the window's films,
// sounds and light in `shooting_star`. Both read this table, so a timeline is
// written once and the two halves cannot drift apart.

/// The skills, in the remote's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Skill {
    ShootingStar,
    Gungnir,
    SevenStars,
    Tesseract,
    Gravitia,
    /// The port's own: not one of the original's (see `original`).
    EventHorizon,
    /// The port's own too.
    Supernova,
}

/// How many skills the remote has.
pub const SKILLS: usize = 7;

/// A skill's power slider (`magic.Power` in the original): a range, where it
/// starts, and whether it moves on a log scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Power {
    pub min: f32,
    pub max: f32,
    pub initial: f32,
    pub factor: bool,
}

impl Power {
    pub fn clamp(&self, value: f32) -> f32 {
        if value.is_nan() {
            return self.initial;
        }
        value.max(self.min).min(self.max)
    }

    /// Where `value` sits on the slider, 0 to 1.
    pub fn position(&self, value: f32) -> f32 {
        let v = self.clamp(value);
        if self.factor {
            ((v / self.min).ln() / (self.max / self.min).ln()).clamp(0.0, 1.0)
        } else {
            ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
        }
    }

    /// The value at slider position `position`, snapped as the original's.
    pub fn at(&self, position: f32) -> f32 {
        let p = position.clamp(0.0, 1.0);
        let v = if self.factor {
            self.min * (self.max / self.min).powf(p)
        } else {
            self.min + (self.max - self.min) * p
        };
        self.snap(v)
    }

    pub fn snap(&self, value: f32) -> f32 {
        let v = if !self.factor {
            (value / 10.0).round() * 10.0
        } else if value < 1.0 {
            (value * 20.0).round() / 20.0
        } else if value < 10.0 {
            (value * 10.0).round() / 10.0
        } else {
            (value * 2.0).round() / 2.0
        };
        self.clamp(v)
    }

    /// One press of a tuning key: a twelfth of the slider up (`dir` 1) or
    /// down (-1), always to a different value while there is one.
    pub fn step(&self, value: f32, dir: i32) -> f32 {
        let from = self.clamp(value);
        let p = self.position(from);
        for i in 1..=60 {
            let next = self.at(p + (dir * i) as f32 / 60.0 * 5.0);
            if (next - from).abs() > f32::EPSILON {
                return next;
            }
        }
        from
    }
}

/// The damage notches (`Tuning.DAMAGE_STEPS`); the last, infinite, is True
/// Erasure, the original's default.
pub const DAMAGE_STEPS: [f32; 23] = [
    2.0,
    3.0,
    4.0,
    5.0,
    6.0,
    8.0,
    10.0,
    12.0,
    15.0,
    20.0,
    25.0,
    30.0,
    40.0,
    50.0,
    60.0,
    80.0,
    100.0,
    150.0,
    200.0,
    300.0,
    500.0,
    1000.0,
    f32::INFINITY,
];

/// The speed slider every skill shares: the whole skill, its film too.
pub const SPEED: Power = Power {
    min: 0.3,
    max: 3.0,
    initial: 1.0,
    factor: true,
};

/// What the player tuned a skill to: its power, the health a hit takes
/// (infinite erases) and how fast it plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    pub power: f32,
    pub damage: f32,
    pub speed: f32,
}

impl Tuning {
    pub fn initial(skill: Skill) -> Tuning {
        Tuning {
            power: skill.power().initial,
            damage: f32::INFINITY,
            speed: SPEED.initial,
        }
    }

    /// Kept inside the skill's ranges, as the server takes it.
    pub fn clamped(self, skill: Skill) -> Tuning {
        let damage = if self.damage.is_nan() || self.damage < DAMAGE_STEPS[0] {
            DAMAGE_STEPS[0]
        } else {
            self.damage
        };
        Tuning {
            power: skill.power().clamp(self.power),
            damage,
            speed: SPEED.clamp(self.speed),
        }
    }

    pub fn erases(&self) -> bool {
        self.damage == f32::INFINITY
    }
}

/// The notch nearest a damage.
pub fn damage_step(damage: f32) -> usize {
    if damage == f32::INFINITY {
        return DAMAGE_STEPS.len() - 1;
    }
    let mut best = 0;
    for (index, step) in DAMAGE_STEPS.iter().enumerate() {
        if step.is_finite() && (step / damage).ln().abs() < (DAMAGE_STEPS[best] / damage).ln().abs() {
            best = index;
        }
    }
    best
}

impl Skill {
    pub const ALL: [Skill; SKILLS] = [
        Skill::ShootingStar,
        Skill::Gungnir,
        Skill::SevenStars,
        Skill::Tesseract,
        Skill::Gravitia,
        Skill::EventHorizon,
        Skill::Supernova,
    ];
    /// The ids of the original's lang file, keys and textures.
    pub const IDS: [&'static str; SKILLS] = ["the_shooting_star", "gungnir", "seven_stars", "tesseract", "gravitia", "event_horizon", "supernova"];

    /// The original's own skill (false for the port's additions).
    pub fn original(self) -> bool {
        !matches!(self, Skill::EventHorizon | Skill::Supernova)
    }

    pub fn id(self) -> &'static str {
        Self::IDS[self.index() as usize]
    }

    pub fn index(self) -> u8 {
        match self {
            Skill::ShootingStar => 0,
            Skill::Gungnir => 1,
            Skill::SevenStars => 2,
            Skill::Tesseract => 3,
            Skill::Gravitia => 4,
            Skill::EventHorizon => 5,
            Skill::Supernova => 6,
        }
    }

    pub fn by_index(index: u8) -> Option<Skill> {
        Self::ALL.get(index as usize).copied()
    }

    pub fn by_id(id: &str) -> Option<Skill> {
        Self::ALL.iter().copied().find(|skill| skill.id() == id)
    }

    pub fn next(self) -> Skill {
        Self::ALL[(self.index() as usize + 1) % Self::ALL.len()]
    }

    /// The name the remote shows: `SS-01 · Railgun`, as the original's.
    pub fn title(self) -> &'static str {
        match self {
            Skill::ShootingStar => "SS-01 · Railgun",
            Skill::Gungnir => "SS-03 · Gungnir",
            Skill::SevenStars => "SS-04 · Seven Stars",
            Skill::Tesseract => "SS-05 · Tesseract",
            Skill::Gravitia => "SS-06 · Gravitia",
            Skill::EventHorizon => "SS-H1 · Event Horizon",
            Skill::Supernova => "SS-H2 · Supernova",
        }
    }

    /// The skill's colour, `0xRRGGBB`.
    pub fn color(self) -> u32 {
        match self {
            Skill::ShootingStar => 0xFF3248,
            Skill::Gungnir => 0xFF5A1F,
            Skill::SevenStars => 0xB8A8FF,
            Skill::Tesseract => 0x7FF0FF,
            Skill::Gravitia => 0xB078FF,
            Skill::EventHorizon => 0xFFA040,
            Skill::Supernova => 0x9FD8FF,
        }
    }

    /// How long the skill runs, in ticks at speed 1.
    pub fn duration(self) -> i32 {
        match self {
            Skill::ShootingStar => 526,
            Skill::Gungnir => 480,
            Skill::SevenStars => 560,
            Skill::Tesseract => 682,
            // The machine hangs in the sky for a hundred seconds after the drop.
            Skill::Gravitia => 2118,
            Skill::EventHorizon => EH_END,
            Skill::Supernova => SN_END,
        }
    }

    /// How long its light lasts in the window, in ticks at speed 1: longer
    /// than the server's part (the original's `SpellFx.duration`): craters
    /// cool, lava dims, the air settles.
    pub fn fx_duration(self) -> i32 {
        match self {
            Skill::ShootingStar => 1386,
            Skill::Gungnir => 1222,
            Skill::SevenStars => 1340,
            Skill::Tesseract => 1262,
            Skill::Gravitia => 2118,
            // The rim cools for a minute.
            Skill::EventHorizon => 1500,
            // The night stays day for a minute; the remnant fades.
            Skill::Supernova => 1600,
        }
    }

    /// The tick its blow lands: the film ends here.
    pub fn impact(self) -> i32 {
        match self {
            Skill::ShootingStar => 352,
            Skill::Gungnir => 322,
            Skill::SevenStars => 290,
            Skill::Tesseract => 462,
            // The blow, under its white flash: the world is sent away unseen.
            Skill::Gravitia => 438,
            Skill::EventHorizon => EH_FLASH,
            Skill::Supernova => SN_BLAST,
        }
    }

    /// The recharge, in ticks: a minute each, a second in creative.
    pub fn cooldown(self) -> u64 {
        1200
    }

    /// How far the remote aims, and how far ahead it lands when the player
    /// looks at the sky.
    pub fn reach(self) -> f64 {
        match self {
            Skill::SevenStars => 320.0,
            _ => 420.0,
        }
    }

    pub fn sky_aim(self) -> f64 {
        match self {
            Skill::SevenStars => 400.0,
            _ => 260.0,
        }
    }

    pub fn power(self) -> Power {
        match self {
            Skill::ShootingStar => Power { min: 10.0, max: 1500.0, initial: 200.0, factor: false },
            Skill::Gungnir => Power { min: 0.1, max: 20.0, initial: 4.0, factor: true },
            Skill::SevenStars => Power { min: 0.1, max: 10.0, initial: 4.0, factor: true },
            Skill::Tesseract => Power { min: 0.05, max: 10.0, initial: 1.0, factor: true },
            Skill::Gravitia => Power { min: 1.0, max: 1.0, initial: 1.0, factor: false },
            Skill::EventHorizon => Power { min: 0.25, max: 6.0, initial: 1.0, factor: true },
            Skill::Supernova => Power { min: 0.25, max: 10.0, initial: 1.0, factor: true },
        }
    }

    /// The tuning page's line for a power, as the original's lang file words it.
    pub fn tune_line(self, power: f32) -> String {
        match self {
            Skill::ShootingStar => format!("the beam {:.0} blocks in radius", power),
            Skill::Gungnir => format!("power {}, the land planed {:.0} blocks round", fmt_power(power), gungnir_radius(power)),
            Skill::SevenStars => format!("power {}, the figure {:.0} blocks across", fmt_power(power), dipper_span(f64::from(power))),
            Skill::Tesseract => format!("power {}, the square {} blocks across", fmt_power(power), tess_zone(tess_scale(power))),
            Skill::Gravitia => "power 1, the whole world".to_owned(),
            Skill::EventHorizon => format!("power {}, a hole {:.0} blocks across", fmt_power(power), eh_radius(power) * 2.0),
            Skill::Supernova => format!("power {}, the land burnt {:.0} blocks round", fmt_power(power), sn_radius(power)),
        }
    }

    /// The default key, as the original binds it (Gravitia, which it adds
    /// later, takes O).
    pub fn default_key(self) -> &'static str {
        match self {
            Skill::ShootingStar => "key.keyboard.k",
            Skill::Gungnir => "key.keyboard.i",
            Skill::SevenStars => "key.keyboard.u",
            Skill::Tesseract => "key.keyboard.y",
            Skill::Gravitia => "key.keyboard.o",
            Skill::EventHorizon => "key.keyboard.b",
            Skill::Supernova => "key.keyboard.v",
        }
    }

    /// The radius players within which see the skill's effects.
    pub fn fx_range(self, power: f32) -> f64 {
        let p = f64::from(power);
        let range = match self {
            Skill::ShootingStar => p + 400.0,
            Skill::Gungnir => gungnir_radius(power) * 2.5 + 200.0,
            Skill::SevenStars => dipper_span(p) + 400.0,
            Skill::Tesseract => f64::from(tess_zone(tess_scale(power))) * 0.75 + 400.0,
            Skill::Gravitia => 1.0e9,
            Skill::EventHorizon => eh_radius(power) * 6.0 + 400.0,
            // The star hangs 300 blocks up: it is seen from far.
            Skill::Supernova => sn_radius(power) * 3.0 + 900.0,
        };
        range.max(512.0)
    }

    /// What the original's item page says, word for word.
    pub fn description(self) -> &'static str {
        match self {
            Skill::ShootingStar => "Calls down an orbital railgun strike from deep space. Fires a devastating beam that disintegrates everything in its radius from build limit to void.",
            Skill::Gungnir => "Launches a hypersonic tungsten needle accelerated around Jupiter. Strikes the target point in eerie silence, leveling the surrounding terrain into a molten crater before the sonic boom hits.",
            Skill::SevenStars => "Projects the Big Dipper constellation onto the terrain. Seven orbital stars bombard each node in sequence, carving glowing trenches and searing the constellation into the earth.",
            Skill::Tesseract => "Summons a fourth-dimensional hypercube over a gridded zone. Severs the terrain and lifts 64 cubic cells into the sky in folding waves, leaving tiered bedrock pits.",
            Skill::Gravitia => "Deploys a planetary ring engine that shatters the world and banishes it into Imaginary Space, reconstructing reality as an endless geometric city in the void.",
            Skill::Supernova => "Lights a star high over the target. It swells into a red giant, falls in on itself and explodes: the night turns to day, the shock front sweeps the ground in a ring, burning grass to ash and sand to glass, and the remnant glows in the sky for a minute.",
            Skill::EventHorizon => "Compresses a satellite into a micro black hole and drops it on the target. Light bends round it, its accretion disk tears the land into orbit, and it evaporates in a white flash, leaving a perfect sphere of emptiness rimmed with molten obsidian.",
        }
    }

    pub fn warning(self) -> &'static str {
        match self {
            Skill::Gravitia => "Your whole world gets sent into Imaginary Space, and it stays there for good. There is no way back.",
            _ => "",
        }
    }
}

fn fmt_power(power: f32) -> String {
    if (power - power.round()).abs() < 0.001 {
        format!("{:.0}", power)
    } else if power < 1.0 {
        format!("{:.2}", power)
    } else {
        format!("{:.1}", power)
    }
}

// ---- shapes both halves draw -------------------------------------------------------------------------------------

/// The original's seeded hash (`hash(x, z)` in Gungnir and Seven Stars), 0 to 1.
pub fn hash(seed: u64, x: i64, z: i64) -> f64 {
    let mut h = (seed as i64)
        ^ x.wrapping_mul(-7_046_029_254_386_353_131)
        ^ z.wrapping_mul(-4_417_276_706_812_531_889);
    h = (h ^ ((h as u64) >> 31) as i64).wrapping_mul(-4_658_895_280_553_007_687);
    h ^= ((h as u64) >> 29) as i64;
    f64::from((((h as u64) >> 11) as f32) / 9.007_199e15_f32)
}

/// SS-01: the beam's radius `t` ticks in: nothing until the impact at 352,
/// whole until 462, then closing by 486.
pub fn beam_radius(t: f64, radius: f64) -> f64 {
    if t < 352.0 {
        0.0
    } else if t < 462.0 {
        radius
    } else {
        let k = ((t - 462.0) / 24.0).clamp(0.0, 1.0);
        radius * (1.0 - k).powf(1.6)
    }
}

/// SS-03: the land planed round the needle, for a power.
pub fn gungnir_radius(power: f32) -> f64 {
    400.0 * gungnir_scale(power)
}

pub fn gungnir_scale(power: f32) -> f64 {
    f64::from(power) / 4.0
}

/// SS-03: how far the shock front has run `t` ticks in.
pub fn gungnir_front(t: f64, radius: f64) -> f64 {
    if t <= 322.0 {
        0.0
    } else {
        radius * ((t - 322.0) / 36.0).powf(0.6)
    }
}

/// SS-04: the Dipper's seven stars (Dubhe to Alkaid), flattened from their
/// right ascension and declination, centred on Megrez, one unit across.
pub const DIPPER_LINKS: [[usize; 2]; 7] = [[0, 1], [1, 2], [2, 3], [3, 0], [3, 4], [4, 5], [5, 6]];
const DIPPER_MAG: [f64; 7] = [1.79, 2.37, 2.44, 3.31, 1.77, 2.23, 1.86];
const DIPPER_RA: [f64; 7] = [11.0621, 11.0307, 11.8972, 12.2571, 12.9005, 13.3988, 13.7923];
const DIPPER_DEC: [f64; 7] = [61.7508, 56.3825, 53.6947, 57.0325, 55.9598, 54.9254, 49.3133];

fn dipper_flat() -> [[f64; 2]; 7] {
    let ra0 = DIPPER_RA.iter().sum::<f64>() / 7.0;
    let dec0 = DIPPER_DEC.iter().sum::<f64>() / 7.0;
    let a0 = (ra0 * 15.0).to_radians();
    let d0 = dec0.to_radians();
    let mut p = [[0.0; 2]; 7];
    for i in 0..7 {
        let a = (DIPPER_RA[i] * 15.0).to_radians();
        let d = DIPPER_DEC[i].to_radians();
        let cosc = d0.sin() * d.sin() + d0.cos() * d.cos() * (a - a0).cos();
        let xi = d.cos() * (a - a0).sin() / cosc;
        let eta = (d0.cos() * d.sin() - d0.sin() * d.cos() * (a - a0).cos()) / cosc;
        p[i] = [-xi, -eta];
    }
    let ax = p[6][0] - p[0][0];
    let az = p[6][1] - p[0][1];
    let turn = -az.atan2(ax);
    let (s, c) = turn.sin_cos();
    let mut min_x = 1.0e9_f64;
    let mut max_x = -1.0e9_f64;
    for q in p.iter_mut() {
        let x = q[0] * c - q[1] * s;
        let z = q[0] * s + q[1] * c;
        *q = [x, z];
        min_x = min_x.min(x);
        max_x = max_x.max(x);
    }
    let w = max_x - min_x;
    let [ox, oz] = p[3];
    for q in p.iter_mut() {
        *q = [(q[0] - ox) / w, (q[1] - oz) / w];
    }
    p
}

/// Which way the figure lies: across the caster's view of the target.
pub fn dipper_turn(caster_x: f64, caster_z: f64, target_x: f64, target_z: f64) -> f64 {
    let hx = caster_x - target_x;
    let hz = caster_z - target_z;
    if hx * hx + hz * hz < 1.0 {
        0.0
    } else {
        hz.atan2(hx) + std::f64::consts::FRAC_PI_2
    }
}

pub fn dipper_span(scale: f64) -> f64 {
    360.0 * scale
}

/// Star `i`'s place, from the target, for a turn and a power.
pub fn dipper_node(i: usize, turn: f64, scale: f64) -> [f64; 2] {
    let flat = dipper_flat();
    let (s, c) = turn.sin_cos();
    let x = flat[i][0] * dipper_span(scale);
    let z = flat[i][1] * dipper_span(scale);
    [x * c - z * s, x * s + z * c]
}

/// The crater a star leaves: the brighter the star, the wider.
pub fn dipper_crater_radius(i: usize, scale: f64) -> f64 {
    (16.0 + (3.4 - DIPPER_MAG[i]) * 12.0) * scale
}

pub fn dipper_crater_depth(i: usize, scale: f64) -> f64 {
    dipper_crater_radius(i, scale) * 0.45
}

/// The tick star `i` lands, and the tick line `k` burns.
pub fn dipper_land_tick(i: usize) -> i32 {
    290 + i as i32 * 16
}

pub fn dipper_link_tick(k: usize) -> i32 {
    398 + k as i32 * 5
}

/// SS-05: the scale a power gives (power 1 is the original's default, 2.5).
pub fn tess_scale(power: f32) -> f64 {
    f64::from(power) / 0.4
}

/// SS-05: the square's side, a multiple of 8 (of 128 once it is large).
pub fn tess_zone(scale: f64) -> i32 {
    let fine = 16.max(((256.0 * scale / 8.0).round() as i32) * 8);
    if fine < 128 {
        fine
    } else {
        ((256.0 * scale / 128.0).round() as i32) * 128
    }
}

/// The square's west (or north) edge, on the chunk grid round the target.
pub fn tess_origin(at: f64, zone: i32) -> i32 {
    (at.floor() as i32 + 8).div_euclid(16) * 16 - zone / 2
}

/// The ring a cell is in: 0 for the four in the middle, 3 round the edge.
pub fn tess_ring(i: i32, j: i32) -> i32 {
    ((f64::from(i) - 3.5).abs().max((f64::from(j) - 3.5).abs())).floor() as i32
}

/// The tick the cubes of ring `ring` lift, in waves from the middle out.
pub fn tess_wave_tick(ring: i32) -> i32 {
    474 + ring * 20 + 8
}

/// How far cell (i, j)'s cube rises, before it is kept under the sky.
pub fn tess_lift(seed: u64, i: i32, j: i32, scale: f64) -> i32 {
    ((90.0 + (hash(seed, i64::from(j * 5 - 1), i64::from(i * 11 + 9)) * 160.0).floor()) * scale).round() as i32
}

// ---- the messages ------------------------------------------------------------------------------------------------

// ---- SS-H1 Event Horizon (the port's own) -----------------------------------------------------------------------
//
// A satellite crushed to a micro black hole falls on the target: it settles
// a little over the ground, its accretion disk spins up and tears the land
// into orbit, the hole eats a sphere of the world, then evaporates in a
// white flash and leaves the sphere empty, its rim fused to obsidian.

/// The hole appears high over the target and starts to fall.
pub const EH_DROP: i32 = 70;
/// It settles over the ground.
pub const EH_LAND: i32 = 130;
/// The disk is spun up; the sphere is eaten outward from here.
pub const EH_FEED: i32 = 160;
/// The sphere is eaten whole.
pub const EH_FULL: i32 = 300;
/// The hole shrinks to nothing.
pub const EH_COLLAPSE: i32 = 330;
/// It evaporates: the flash.
pub const EH_FLASH: i32 = 346;
pub const EH_END: i32 = 560;

/// The sphere's radius at a power.
pub fn eh_radius(power: f32) -> f64 {
    (14.0 * f64::from(power).sqrt()).clamp(6.0, 48.0)
}

/// How far the hole pulls: everything loose, the living and the ground's
/// top within it, harder the nearer and the more it has eaten.
pub fn eh_pull(power: f32) -> f64 {
    eh_radius(power) * 6.0 + 48.0
}

/// Where the hole sits once it has settled: a quarter of its sphere over the ground.
pub fn eh_center(target: [f64; 3], power: f32) -> [f64; 3] {
    [target[0], target[1] + eh_radius(power) * 0.25, target[2]]
}

/// How much of the sphere is eaten (0 to 1), by tick.
pub fn eh_eaten(t: f64) -> f64 {
    let k = ((t - f64::from(EH_FEED)) / f64::from(EH_FULL - EH_FEED)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

/// The hole's centre at a tick: high over the target, falling, settled.
pub fn eh_hole(target: [f64; 3], power: f32, t: f64) -> [f64; 3] {
    let c = eh_center(target, power);
    let k = ((t - f64::from(EH_DROP)) / f64::from(EH_LAND - EH_DROP)).clamp(0.0, 1.0);
    // Faster and faster, braking hard at the end.
    let fall = k * k * (3.0 - 2.0 * k) * 0.3 + (1.0 - (1.0 - k).powi(3)) * 0.7;
    [c[0], c[1] + 220.0 * (1.0 - fall), c[2]]
}

// ---- SS-H2 Supernova (the port's own) ---------------------------------------------------------------------------
//
// A star is lit 300 blocks over the target, swells into a red giant, falls
// in on itself and explodes; the blast reaches the ground and its front
// sweeps out in a ring, burning the land as it goes.

/// The star is lit.
pub const SN_LIGHT: i32 = 30;
/// It starts to swell.
pub const SN_SWELL: i32 = 60;
/// At its largest: it starts to fall in.
pub const SN_PEAK: i32 = 220;
/// It explodes.
pub const SN_BLAST: i32 = 250;
/// The blast reaches the ground under it.
pub const SN_HIT: i32 = 262;
pub const SN_END: i32 = 420;
/// How high over the target the star hangs.
pub const SN_HEIGHT: f64 = 300.0;

/// How far the ground burns round the target.
pub fn sn_radius(power: f32) -> f64 {
    (90.0 * f64::from(power).sqrt()).clamp(30.0, 280.0)
}

pub fn sn_star(target: [f64; 3]) -> [f64; 3] {
    [target[0], target[1] + SN_HEIGHT, target[2]]
}

/// Ticks the ground front takes to reach its edge.
pub fn sn_ring_ticks(power: f32) -> f64 {
    20.0 + sn_radius(power) / 3.0
}

/// The ground front's radius at a tick (0 before it lands), fast at first, slowing.
pub fn sn_ring(t: f64, power: f32) -> f64 {
    let k = ((t - f64::from(SN_HIT)) / sn_ring_ticks(power)).clamp(0.0, 1.0);
    if t < f64::from(SN_HIT) {
        return 0.0;
    }
    sn_radius(power) * (1.0 - (1.0 - k).powf(1.8))
}

/// The window to the server.
pub const CMD_SELECT: u8 = 2;
pub const CMD_TUNE: u8 = 4;
/// The window's switches: `[FLAGS, bits]`, bit 0 = spells may change blocks.
pub const CMD_FLAGS: u8 = 9;
/// The server to the window.
pub const FX_STATE: u8 = 3;
pub const FX_START: u8 = 5;
pub const FX_SELECT: u8 = 6;
pub const FX_HELLO: u8 = 7;
/// A level that is Imaginary Space: seed (u32 LE), then its dimension's name.
pub const FX_VOID: u8 = 8;
/// The trailer director on (1) or off (0): `[FX_TRAILER, on]`.
pub const FX_TRAILER: u8 = 10;

/// A spell as its effects are sent: who, what, where, and how it was tuned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Start {
    pub skill: Skill,
    pub mine: bool,
    pub origin: [f64; 3],
    pub target: [f64; 3],
    pub yaw: f32,
    pub seed: u64,
    pub power: f32,
    pub speed: f32,
}

pub fn encode_start(start: &Start) -> Vec<u8> {
    let mut out = Vec::with_capacity(72);
    out.push(FX_START);
    out.push(start.skill.index());
    out.push(u8::from(start.mine));
    for v in start.origin.iter().chain(start.target.iter()) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&start.yaw.to_le_bytes());
    out.extend_from_slice(&start.seed.to_le_bytes());
    out.extend_from_slice(&start.power.to_le_bytes());
    out.extend_from_slice(&start.speed.to_le_bytes());
    out
}

pub fn decode_start(data: &[u8]) -> Option<Start> {
    if data.len() < 71 || data[0] != FX_START {
        return None;
    }
    let f64_at = |at: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&data[at..at + 8]);
        f64::from_le_bytes(b)
    };
    let f32_at = |at: usize| {
        let mut b = [0u8; 4];
        b.copy_from_slice(&data[at..at + 4]);
        f32::from_le_bytes(b)
    };
    let mut seed = [0u8; 8];
    seed.copy_from_slice(&data[55..63]);
    Some(Start {
        skill: Skill::by_index(data[1])?,
        mine: data[2] != 0,
        origin: [f64_at(3), f64_at(11), f64_at(19)],
        target: [f64_at(27), f64_at(35), f64_at(43)],
        yaw: f32_at(51),
        seed: u64::from_le_bytes(seed),
        power: f32_at(63),
        speed: f32_at(67),
    })
}

/// The window's tuning of every skill: `[TUNE, n, n x (power, damage, speed)]`.
pub fn encode_tunes(tunes: &[Tuning; SKILLS]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + 12 * SKILLS);
    out.push(CMD_TUNE);
    out.push(SKILLS as u8);
    for tune in tunes {
        out.extend_from_slice(&tune.power.to_le_bytes());
        out.extend_from_slice(&tune.damage.to_le_bytes());
        out.extend_from_slice(&tune.speed.to_le_bytes());
    }
    out
}

pub fn decode_tunes(data: &[u8]) -> Option<[Tuning; SKILLS]> {
    // An older window's five are read too: the rest keep their defaults.
    let n = usize::from(*data.get(1)?);
    if data[0] != CMD_TUNE || n < 5 || data.len() < 2 + 12 * n {
        return None;
    }
    let word = |at: usize| f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    let mut tunes = Skill::ALL.map(Tuning::initial);
    for (i, skill) in Skill::ALL.iter().enumerate().take(n) {
        let at = 2 + i * 12;
        tunes[i] = Tuning { power: word(at), damage: word(at + 4), speed: word(at + 8) }.clamped(*skill);
    }
    Some(tunes)
}

/// The recharges, as the server tells a player: `[STATE, n, n x (left, total)]` in ticks.
pub fn encode_state(left: &[(u32, u32); SKILLS]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + 8 * SKILLS);
    out.push(FX_STATE);
    out.push(SKILLS as u8);
    for (remaining, total) in left {
        out.extend_from_slice(&remaining.to_le_bytes());
        out.extend_from_slice(&total.to_le_bytes());
    }
    out
}

pub fn decode_state(data: &[u8]) -> Option<[(u32, u32); SKILLS]> {
    let n = usize::from(*data.get(1)?);
    if data[0] != FX_STATE || n < 5 || data.len() < 2 + 8 * n {
        return None;
    }
    let word = |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    let mut out = [(0, 1); SKILLS];
    for (i, slot) in out.iter_mut().enumerate().take(n) {
        *slot = (word(2 + i * 8), word(6 + i * 8).max(1));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skills_keep_the_originals_ids_and_order() {
        assert_eq!(Skill::ALL.len(), 7);
        assert_eq!(Skill::ShootingStar.id(), "the_shooting_star");
        assert_eq!(Skill::by_id("seven_stars"), Some(Skill::SevenStars));
        assert_eq!(Skill::Gravitia.next(), Skill::EventHorizon);
        assert_eq!(Skill::EventHorizon.next(), Skill::Supernova);
        assert_eq!(Skill::Supernova.next(), Skill::ShootingStar);
        assert!(Skill::ALL[..5].iter().all(|s| s.original()) && !Skill::EventHorizon.original() && !Skill::Supernova.original());
    }

    #[test]
    fn the_originals_defaults() {
        assert_eq!(Tuning::initial(Skill::ShootingStar).power, 200.0);
        assert!(Tuning::initial(Skill::Gungnir).erases());
        assert_eq!(gungnir_radius(4.0), 400.0);
        assert_eq!(tess_zone(tess_scale(1.0)), 640);
        assert_eq!(dipper_span(4.0), 1440.0);
    }

    #[test]
    fn the_beam_opens_at_the_impact_and_closes() {
        assert_eq!(beam_radius(351.0, 200.0), 0.0);
        assert_eq!(beam_radius(400.0, 200.0), 200.0);
        assert!((beam_radius(480.0, 200.0) - 21.76).abs() < 0.05);
        assert_eq!(beam_radius(486.0, 200.0), 0.0);
    }

    #[test]
    fn the_rings_go_out_from_the_middle() {
        assert_eq!(tess_ring(3, 4), 0);
        assert_eq!(tess_ring(0, 0), 3);
        assert_eq!(tess_ring(7, 2), 3);
        assert_eq!(tess_ring(2, 5), 1);
    }

    #[test]
    fn megrez_is_the_middle_and_the_figure_is_one_span_wide() {
        let node = dipper_node(3, 0.0, 1.0);
        assert!(node[0].abs() < 1e-9 && node[1].abs() < 1e-9);
        let xs: Vec<f64> = (0..7).map(|i| dipper_node(i, 0.0, 1.0)[0]).collect();
        let span = xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
        assert!((span - 360.0).abs() < 1e-6);
    }

    #[test]
    fn messages_read_back() {
        let start = Start {
            skill: Skill::Gungnir,
            mine: true,
            origin: [1.5, 64.0, -3.25],
            target: [100.5, 70.0, 2000.75],
            yaw: 45.0,
            seed: 0xDEAD_BEEF_1234,
            power: 4.0,
            speed: 1.5,
        };
        assert_eq!(decode_start(&encode_start(&start)), Some(start));
        let tunes = [Tuning::initial(Skill::ShootingStar); SKILLS];
        assert!(decode_tunes(&encode_tunes(&tunes)).is_some());
        let state = [(10, 1200), (0, 1200), (0, 20), (5, 1200), (0, 1200), (7, 1200), (3, 1200)];
        assert_eq!(decode_state(&encode_state(&state)), Some(state));
    }

    #[test]
    fn a_power_step_moves_and_stays_in_range() {
        let p = Skill::ShootingStar.power();
        let up = p.step(200.0, 1);
        assert!(up > 200.0 && up <= 1500.0);
        assert_eq!(p.step(1500.0, 1), 1500.0);
        let g = Skill::Gungnir.power();
        assert!(g.step(4.0, -1) < 4.0);
    }
}
