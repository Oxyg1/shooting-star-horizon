// The server's half of the Stellar Remote: casting, the spells' timelines, the
// land they cut and the life they take, ported from the original's
// `StarEngine`, `ShootingStar`, `Gungnir`, `SevenStars`, `Carving` and
// `Erasure` (1.3.4, the last readable build) and its Tesseract (1.4.4), with
// the sizes 1.5.2 still uses.
//
// Everything here runs on the server thread: the remote's right click
// (`use_item`), the level's tick (`level_tick_post`) and the server's own
// tick. The window only hears what it must draw and play.
use super::shooting_star::{CHANNEL, ID};
use super::shooting_star_city as city;
use super::shooting_star_skills::{self as sk, Skill, Tuning};
use crate::game::native_world_edit::{NativeBlockEdit, NativeBlockRun, NativeHeightmap};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

/// The chunks every carving may hold loaded at once, all spells together:
/// Horizon's chunk tickets are meant for dozens, not thousands.
const MAX_HOLDS: usize = 96;
/// How long a second cast has to confirm Gravitia, in ticks.
const CONFIRM_TICKS: u64 = 200;

const AIR: &str = "minecraft:air";
const STAR_CORE: &str = "shooting_star_demo:star_core";
const STAR_TRACE: &str = "shooting_star_demo:star_trace";
const HULL: &str = "shooting_star_demo:gungnir_hull";
const SEAM: &str = "shooting_star_demo:gungnir_seam";

// ---- who casts what ----------------------------------------------------------------------------------------------

/// What the server keeps of a player's remote.
#[derive(Clone, Debug)]
struct Remote {
    selected: Skill,
    tunes: [Tuning; sk::SKILLS],
    /// The server tick each skill is ready again.
    ready: [u64; sk::SKILLS],
    /// When the player was warned about Gravitia, for the second cast.
    confirm: Option<u64>,
    /// Whether this player's spells may change blocks (the window's "Land").
    terrain: bool,
}

impl Default for Remote {
    fn default() -> Self {
        Remote {
            selected: Skill::ShootingStar,
            tunes: Skill::ALL.map(Tuning::initial),
            ready: [0; sk::SKILLS],
            confirm: None,
            terrain: true,
        }
    }
}

thread_local! {
    static REMOTES: RefCell<HashMap<u128, Remote>> = RefCell::new(HashMap::new());
    static SPELLS: RefCell<Vec<Spell>> = const { RefCell::new(Vec::new()) };
    /// The server's ticks since it started.
    static NOW: Cell<u64> = const { Cell::new(0) };
    /// The world the spells belong to: a new world drops the old one's.
    static WORLD: RefCell<Option<std::path::PathBuf>> = const { RefCell::new(None) };
    /// Chunks the carvings hold loaded right now.
    static HOLDS: Cell<usize> = const { Cell::new(0) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    /// The spell whose tick is running, so `apply` knows whose edit it is.
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
    /// The land each player's last spells changed, for `/tss undo`.
    static JOURNALS: RefCell<HashMap<u128, Vec<Journal>>> = RefCell::new(HashMap::new());
    /// The undos being carried out, a few chunks a tick.
    static RESTORES: RefCell<Vec<Restore>> = const { RefCell::new(Vec::new()) };
}

fn now() -> u64 {
    NOW.with(Cell::get)
}

fn with_remote<R>(uuid: u128, f: impl FnOnce(&mut Remote) -> R) -> R {
    REMOTES.with(|remotes| f(remotes.borrow_mut().entry(uuid).or_default()))
}

/// The server's own tick (the `tick` hook, off the window): the clock, and a
/// fresh start when another world opens.
pub fn server_tick(game: &mut super::Game<'_>) {
    NOW.with(|n| n.set(n.get() + 1));
    let world = game.native_world_dir();
    let changed = WORLD.with(|w| {
        let mut w = w.borrow_mut();
        if *w != world {
            *w = world.clone();
            true
        } else {
            false
        }
    });
    if changed {
        SPELLS.with(|s| s.borrow_mut().clear());
        JOURNALS.with(|j| j.borrow_mut().clear());
        RESTORES.with(|r| r.borrow_mut().clear());
        REMOTES.with(|r| r.borrow_mut().clear());
        HOLDS.with(|h| h.set(0));
        city::open_world(world.as_deref());
    }
}

/// A player joined: ask their window for its selection and tuning.
pub fn player_joined(game: &mut super::Game<'_>, uuid: u128) {
    let _ = game.native_net_send_to_player(uuid, CHANNEL, &[sk::FX_HELLO]);
    for (dimension, seed) in city::banished_levels() {
        let _ = game.native_net_send_to_player(uuid, CHANNEL, &city::void_message(&dimension, seed));
    }
    send_state(game, uuid);
}

/// A message from a player's window: their selection or their tuning.
pub fn on_client_message(player: u128, channel: &str, data: &[u8]) {
    if channel != super::shooting_star::CH_CMD || data.is_empty() {
        return;
    }
    match data[0] {
        sk::CMD_SELECT if data.len() >= 2 => {
            if let Some(skill) = Skill::by_index(data[1]) {
                with_remote(player, |r| r.selected = skill);
            }
        }
        sk::CMD_TUNE => {
            if let Some(tunes) = sk::decode_tunes(data) {
                with_remote(player, |r| r.tunes = tunes);
            }
        }
        // Bit 0: spells may change blocks. Older servers ignore this byte.
        sk::CMD_FLAGS if data.len() >= 2 => with_remote(player, |r| r.terrain = data[1] & 1 != 0),
        _ => {}
    }
}

/// Move a player's selection on one skill (`/tss next`).
pub fn cycle(uuid: u128) -> Skill {
    with_remote(uuid, |r| {
        r.selected = r.selected.next();
        r.selected
    })
}

/// Tell a player how long each of their skills still recharges.
fn send_state(game: &mut super::Game<'_>, uuid: u128) {
    let creative = game
        .native_server_players()
        .iter()
        .any(|p| p.uuid == uuid && p.game_mode == 1);
    let t = now();
    let left = with_remote(uuid, |r| {
        let mut left = [(0u32, 1u32); sk::SKILLS];
        for (i, skill) in Skill::ALL.iter().enumerate() {
            let total = if creative { 20 } else { skill.cooldown() };
            left[i] = (r.ready[i].saturating_sub(t).min(u64::from(u32::MAX)) as u32, total as u32);
        }
        left
    });
    if let Err(error) = game.native_net_send_to_player(uuid, CHANNEL, &sk::encode_state(&left)) {
        log::debug!("[{ID}] the recharge state waits: {error}");
    }
}

fn deny(game: &mut super::Game<'_>, uuid: u128, message: String) {
    game.native_action_bar(uuid, crate::text::Text::literal(message));
}

/// The remote's right click (`use_item`, on the server): sneak cycles the
/// selection, anything else casts it, as the original's `MagicItem.use`.
pub fn use_remote(game: &mut super::Game<'_>) {
    let Some(player) = game.native_acting_player() else {
        return;
    };
    if game.native_sneaking() {
        let next = with_remote(player.uuid, |r| {
            r.selected = r.selected.next();
            r.selected
        });
        let _ = game.native_net_send_to_player(player.uuid, CHANNEL, &[sk::FX_SELECT, next.index()]);
        deny(game, player.uuid, format!("✦ {}", next.title()));
        return;
    }
    if player.game_mode == 3 || player.health <= 0.0 {
        return;
    }
    let (skill, tune) = with_remote(player.uuid, |r| (r.selected, r.tunes[r.selected.index() as usize]));
    cast(game, &player, skill, tune);
}

/// Cast `skill` for `player`, tuned: the original's `Casting.cast`.
pub fn cast(game: &mut super::Game<'_>, player: &super::NativeServerPlayer, skill: Skill, tune: Tuning) {
    let tune = tune.clamped(skill);
    let t = now();
    let running = SPELLS.with(|s| s.borrow().iter().any(|spell| spell.skill == skill && spell.caster == player.uuid));
    if running {
        deny(game, player.uuid, format!("{} is already on its way", skill.title()));
        return;
    }
    let ready = with_remote(player.uuid, |r| r.ready[skill.index() as usize]);
    if t < ready {
        deny(game, player.uuid, format!("{} is recharging — {}s", skill.title(), (ready - t + 19) / 20));
        return;
    }
    if skill == Skill::Gravitia {
        if city::banished(&player.dimension) {
            deny(game, player.uuid, "This world is already in Imaginary Space".to_owned());
            return;
        }
        // The original only warns on its menu: here the remote asks twice.
        let confirmed = with_remote(player.uuid, |r| {
            let ok = r.confirm.is_some_and(|at| t <= at + CONFIRM_TICKS);
            r.confirm = if ok { None } else { Some(t) };
            ok
        });
        if !confirmed {
            deny(
                game,
                player.uuid,
                format!("⚠ {} Right-click again within 10s to cast it.", Skill::Gravitia.warning()),
            );
            return;
        }
    }
    let (target, ground_known) = aim_ground(game, player, skill.reach(), skill.sky_aim());
    let seed = spell_seed(player.uuid, t);
    let (min_y, height) = game.native_world_height();
    let mut spell = Spell::new(skill, player, target, seed, tune, min_y, min_y + height - 1);
    spell.terrain = with_remote(player.uuid, |r| r.terrain);
    if spell.terrain && skill != Skill::Gravitia {
        open_journal(&spell);
    }
    spell.ground_known = ground_known;
    log::info!(
        "[{ID}] {} cast by {} at {:.0} {:.0} {:.0}, power {}, speed {}",
        skill.title(),
        player.name,
        target[0],
        target[1],
        target[2],
        tune.power,
        tune.speed
    );
    let start = sk::Start {
        skill,
        mine: false,
        origin: player.position,
        target,
        yaw: player.yaw,
        seed,
        power: tune.power,
        speed: tune.speed,
    };
    broadcast_start(game, &player.dimension, player.uuid, start, skill.fx_range(tune.power));
    SPELLS.with(|s| s.borrow_mut().push(spell));
    let wait = if player.game_mode == 1 { 20 } else { skill.cooldown() };
    with_remote(player.uuid, |r| r.ready[skill.index() as usize] = t + wait);
    send_state(game, player.uuid);
}

fn spell_seed(uuid: u128, t: u64) -> u64 {
    let mut h = (uuid as u64) ^ ((uuid >> 64) as u64) ^ t.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^ (h >> 29)
}

/// The spell's effects, to the caster and everyone within `range` of it.
fn broadcast_start(game: &mut super::Game<'_>, dimension: &str, caster: u128, start: sk::Start, range: f64) {
    let mine = sk::Start { mine: true, ..start };
    if let Err(error) = game.native_net_send_to_player(caster, CHANNEL, &sk::encode_start(&mine)) {
        log::info!("[{ID}] the spell's effects wait: {error}");
    }
    let others = sk::encode_start(&start);
    for other in game.native_server_players() {
        if other.uuid == caster || other.dimension != dimension {
            continue;
        }
        let d = (other.position[0] - start.target[0]).hypot(other.position[2] - start.target[2]);
        if d <= range {
            let _ = game.native_net_send_to_player(other.uuid, CHANNEL, &others);
        }
    }
}

/// Where the remote lands: the creature under the crosshair, the block the
/// view meets, or the ground below a point far ahead when the player looks
/// up (`SpellEngine.aimGround`).
fn aim_ground(game: &mut super::Game<'_>, player: &super::NativeServerPlayer, reach: f64, sky: f64) -> ([f64; 3], bool) {
    let eye = [player.position[0], player.position[1] + 1.62, player.position[2]];
    let look = view_vector(player.yaw, player.pitch);
    let hit = game.native_raycast(reach as f32);
    let ray_end = match &hit {
        Some(h) => ((h.x - eye[0]).powi(2) + (h.y - eye[1]).powi(2) + (h.z - eye[2]).powi(2)).sqrt(),
        None => reach,
    };
    // A creature along the ray before the block.
    let mut creature: Option<(f64, [f64; 3])> = None;
    for mob in game.native_mobs(&player.dimension) {
        let mid = [mob.pos[0], mob.pos[1] + 0.9, mob.pos[2]];
        let rel = [mid[0] - eye[0], mid[1] - eye[1], mid[2] - eye[2]];
        let along = rel[0] * look[0] + rel[1] * look[1] + rel[2] * look[2];
        if along <= 0.0 || along > ray_end {
            continue;
        }
        let off = [rel[0] - look[0] * along, rel[1] - look[1] * along, rel[2] - look[2] * along];
        if off[0] * off[0] + off[1] * off[1] + off[2] * off[2] <= 1.6 * 1.6 && creature.is_none_or(|(best, _)| along < best) {
            creature = Some((along, mob.pos));
        }
    }
    let point = if let Some((_, at)) = creature {
        at
    } else if let Some(h) = hit {
        [h.x - look[0] * 0.05, h.y - look[1] * 0.05, h.z - look[2] * 0.05]
    } else if look[1] < 0.0 {
        [eye[0] + look[0] * reach, eye[1] + look[1] * reach, eye[2] + look[2] * reach]
    } else {
        let flat = look[0].hypot(look[2]);
        let (dx, dz) = if flat < 1.0e-2 { (0.0, 1.0) } else { (look[0] / flat, look[2] / flat) };
        let (min_y, height) = game.native_world_height();
        [eye[0] + dx * sky, f64::from(min_y + height - 1), eye[2] + dz * sky]
    };
    let x = point[0].floor() as i32;
    let z = point[2].floor() as i32;
    let ground = game
        .native_surface_heights(&player.dimension, &[(x, z)], NativeHeightmap::MotionBlockingNoLeaves)
        .ok()
        .and_then(|h| h.first().copied().flatten());
    match ground {
        Some(top) => ([point[0], f64::from((point[1].floor() as i32).min(top)), point[2]], true),
        // Land not loaded yet: the caster's height until it is.
        None => ([point[0], player.position[1].floor().min(point[1].floor()), point[2]], false),
    }
}

/// The direction a player looks (yaw 0 faces south, +Z; pitch 90 is down).
pub fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (sy, cy) = f64::from(yaw).to_radians().sin_cos();
    let (sp, cp) = f64::from(pitch).to_radians().sin_cos();
    [-sy * cp, -sp, cy * cp]
}

// ---- the spells' timelines ----------------------------------------------------------------------------------------

/// One spell on its way, as `ActiveSpell` is.
struct Spell {
    id: u64,
    skill: Skill,
    caster: u128,
    caster_name: String,
    dimension: String,
    origin: [f64; 3],
    target: [f64; 3],
    seed: u64,
    tune: Tuning,
    age: i32,
    clock: f64,
    min_y: i32,
    max_y: i32,
    /// The 5x5 column under a creative caster that could not be moved.
    spared: Option<[i32; 3]>,
    carvings: Vec<Carving>,
    /// The tick each mob and player was last hit, for the hurt cooldown.
    mob_hits: HashMap<u32, i32>,
    player_hits: HashMap<u128, i32>,
    /// Whether the target's ground was read (its land may load later).
    ground_known: bool,
    /// Whether the spell may change blocks (off in the caster's safe mode).
    terrain: bool,
    /// The chunks round the target held loaded for the spell's whole life.
    held: Vec<[i32; 2]>,
    kind: Kind,
}

enum Kind {
    Star,
    Gungnir(Box<GungnirShape>),
    Seven(Box<SevenState>),
    Tess(Box<TessState>),
    Grav,
    Hole(Box<HoleState>),
    Nova(Box<NovaState>),
}

fn make_kind(skill: Skill, origin: [f64; 3], target: [f64; 3], seed: u64, tune: Tuning, min_y: i32) -> Kind {
    match skill {
        Skill::ShootingStar => Kind::Star,
        Skill::Gungnir => Kind::Gungnir(Box::new(GungnirShape::new(target, seed, tune.power, min_y))),
        Skill::SevenStars => Kind::Seven(Box::new(SevenState::new(origin, target, tune.power))),
        Skill::Tesseract => Kind::Tess(Box::new(TessState::new(target, tune.power, min_y))),
        Skill::Gravitia => Kind::Grav,
        Skill::EventHorizon => Kind::Hole(Box::new(HoleState::new(target, tune.power))),
        Skill::Supernova => Kind::Nova(Box::new(NovaState { burnt: 0.0 })),
    }
}

impl Spell {
    fn new(
        skill: Skill,
        caster: &super::NativeServerPlayer,
        target: [f64; 3],
        seed: u64,
        tune: Tuning,
        min_y: i32,
        max_y: i32,
    ) -> Spell {
        let id = NEXT_ID.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        let kind = make_kind(skill, caster.position, target, seed, tune, min_y);
        Spell {
            id,
            skill,
            caster: caster.uuid,
            caster_name: caster.name.clone(),
            dimension: caster.dimension.clone(),
            origin: caster.position,
            target,
            seed,
            tune,
            age: 0,
            clock: 0.0,
            min_y,
            max_y,
            spared: None,
            carvings: Vec::new(),
            mob_hits: HashMap::new(),
            player_hits: HashMap::new(),
            ground_known: true,
            terrain: true,
            held: Vec::new(),
            kind,
        }
    }

    fn owner(&self) -> String {
        format!("shooting_star_demo:spell/{}", self.id)
    }

    fn finished(&self) -> bool {
        let busy = match &self.kind {
            Kind::Tess(state) => !state.jobs.is_empty() || state.current.is_some(),
            _ => false,
        };
        self.age > self.skill.duration() && self.carvings.iter().all(Carving::done) && !busy
    }

    /// The real ticks a timeline span takes at this spell's speed.
    fn real_ticks(&self, timeline: i32) -> i32 {
        ((timeline as f32 / self.tune.speed).round() as i32).max(1)
    }
}

/// The level's tick (`level_tick_post`): the spells cast in this level move
/// on, and Imaginary Space spreads.
pub fn level_tick(game: &mut super::Game<'_>, dimension: &str) {
    let mut spells = SPELLS.with(|s| std::mem::take(&mut *s.borrow_mut()));
    for spell in spells.iter_mut().filter(|s| s.dimension == dimension) {
        let caster = game
            .native_server_players()
            .into_iter()
            .find(|p| p.uuid == spell.caster && p.dimension == spell.dimension);
        if spell.age == 0 && spell.held.is_empty() {
            // The target's land stays loaded while the spell needs it.
            let owner = spell.owner();
            let centre = [(spell.target[0].floor() as i32) >> 4, (spell.target[2].floor() as i32) >> 4];
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let chunk = [centre[0] + dx, centre[1] + dz];
                    if hold(game, &owner, chunk) {
                        spell.held.push(chunk);
                    }
                }
            }
        }
        if !spell.ground_known && spell.age < spell.skill.impact() {
            refine_ground(game, spell);
        }
        let until = (spell.clock + 1.0e-6).floor() as i32;
        while spell.age <= until && spell.age <= spell.skill.duration() {
            let t = spell.age;
            enter(spell);
            on_tick(game, spell, caster.as_ref(), t);
            leave();
            spell.age += 1;
        }
        spell.clock += f64::from(spell.tune.speed);
        if spell.age > spell.skill.duration() {
            // Lingering work, as `Carving.tickLingering`: the cut goes on
            // after the spell's own timeline ends.
            enter(spell);
            after_end(game, spell);
            leave();
        }
    }
    let mut kept = Vec::with_capacity(spells.len());
    for mut spell in spells {
        if spell.dimension == dimension && spell.finished() {
            for carving in spell.carvings.iter_mut() {
                carving.release_all(game);
            }
            let owner = spell.owner();
            if let Kind::Tess(state) = &mut spell.kind {
                state.release(game, &owner);
            }
            for chunk in std::mem::take(&mut spell.held) {
                release(game, &owner, chunk);
            }
            close_journal(&spell);
            log::info!("[{ID}] {} has run its course", spell.skill.title());
            continue;
        }
        kept.push(spell);
    }
    SPELLS.with(|s| {
        let mut s = s.borrow_mut();
        kept.append(&mut s);
        *s = kept;
    });
    restore_tick(game, dimension);
    city::level_tick(game, dimension);
}

/// The ground under a target far out in land not loaded at the cast, read
/// once its chunk has loaded.
fn refine_ground(game: &mut super::Game<'_>, spell: &mut Spell) {
    let x = spell.target[0].floor() as i32;
    let z = spell.target[2].floor() as i32;
    let Ok(heights) = game.native_surface_heights(&spell.dimension, &[(x, z)], NativeHeightmap::MotionBlockingNoLeaves) else {
        return;
    };
    let Some(Some(top)) = heights.first().copied() else {
        return;
    };
    spell.target[1] = f64::from(top);
    spell.kind = make_kind(spell.skill, spell.origin, spell.target, spell.seed, spell.tune, spell.min_y);
    spell.ground_known = true;
}

fn after_end(game: &mut super::Game<'_>, spell: &mut Spell) {
    let dim = spell.dimension.clone();
    let mut cut = Vec::new();
    for carving in spell.carvings.iter_mut() {
        if !carving.done() {
            cut.extend(carving.tick(game, &dim, f64::MAX));
        }
    }
    repaint_cores(game, spell, &cut);
    if let Kind::Tess(_) = spell.kind {
        tess_work(game, spell);
    }
}

fn on_tick(game: &mut super::Game<'_>, spell: &mut Spell, caster: Option<&super::NativeServerPlayer>, t: i32) {
    if let Some(c) = caster
        && t % 10 == 0
    {
        ward(game, c);
    }
    match spell.skill {
        Skill::ShootingStar => star_tick(game, spell, caster, t),
        Skill::Gungnir => gungnir_tick(game, spell, t),
        Skill::SevenStars => seven_tick(game, spell, caster, t),
        Skill::Tesseract => tess_tick(game, spell, caster, t),
        Skill::Gravitia => grav_tick(game, spell, t),
        Skill::EventHorizon => hole_tick(game, spell, caster, t),
        Skill::Supernova => nova_tick(game, spell, t),
    }
}

// ---- SS-01 The Shooting Star ------------------------------------------------------------------------------------

fn star_tick(game: &mut super::Game<'_>, spell: &mut Spell, caster: Option<&super::NativeServerPlayer>, t: i32) {
    let radius = f64::from(spell.tune.power);
    if t == 31 {
        if let Some(c) = caster {
            let ticks = spell.real_ticks(495);
            spell.spared = evacuate(game, spell, c, radius + 3.0, ticks);
        }
    }
    let dim = spell.dimension.clone();
    if t == 352 {
        let shape = Shape::Cylinder { cx: spell.target[0], cz: spell.target[2], r: radius };
        let carving = Carving::new(spell.owner(), shape, spell.target, radius, true, 12, spell.spared, spell.min_y, spell.max_y);
        spell.carvings.push(carving);
    }
    for carving in spell.carvings.iter_mut() {
        if !carving.done() {
            carving.tick(game, &dim, f64::MAX);
        }
    }
    if (352..486).contains(&t) && (t - 352) % 2 == 0 {
        let r = sk::beam_radius(f64::from(t), radius);
        if r > 0.5 {
            let (cx, cz) = (spell.target[0], spell.target[2]);
            let loose = t == 352 || (t - 352) % 10 == 0;
            strike(game, spell, |p| (p[0] - cx).powi(2) + (p[2] - cz).powi(2) <= r * r, loose.then_some((cx, cz, r)), None);
        }
    }
}

// ---- SS-03 Gungnir ----------------------------------------------------------------------------------------------

/// Gungnir's crater, as the original's `Gungnir` shapes it.
struct GungnirShape {
    tx: f64,
    tz: f64,
    seed: u64,
    scale: f64,
    ground: i32,
    min_y: i32,
    radius: f64,
    crater_r: f64,
    crater_depth: f64,
    driven: f64,
    fissure_depth: f64,
    ramp: f64,
    spire_r: f64,
    fin_span: f64,
    fin_height: i32,
    rim_phase: [f64; 3],
}

impl GungnirShape {
    fn new(target: [f64; 3], seed: u64, power: f32, min_y: i32) -> GungnirShape {
        let scale = sk::gungnir_scale(power);
        let spire = scale.sqrt().min(1.0);
        let ground = target[1].floor() as i32 - 1;
        let driven = 20.0 * scale;
        let mut rim_phase = [0.0; 3];
        for (i, phase) in rim_phase.iter_mut().enumerate() {
            *phase = ((seed as i64) >> (i * 11)).rem_euclid(6283) as f64 / 1000.0;
        }
        GungnirShape {
            tx: target[0],
            tz: target[2],
            seed,
            scale,
            ground,
            min_y,
            radius: 400.0 * scale,
            crater_r: 88.0 * scale,
            // 1.5.2 keeps the bowl above the world's floor.
            crater_depth: (108.0 * scale).min((f64::from(ground) - driven - f64::from(min_y + 2)).max(0.0)),
            driven,
            fissure_depth: 7.0 * scale.max(0.5),
            ramp: 36.0 * scale,
            spire_r: 6.5 * spire,
            fin_span: 20.0 * spire,
            fin_height: (60.0 * spire).round() as i32,
            rim_phase,
        }
    }

    fn rim(&self, angle: f64) -> f64 {
        self.radius
            + self.scale
                * (6.0 * (3.0 * angle + self.rim_phase[0]).sin()
                    + 4.0 * (7.0 * angle + self.rim_phase[1]).sin()
                    + 2.4 * (13.0 * angle + self.rim_phase[2]).sin())
    }

    fn spire(&self, x: i32, z: i32) -> bool {
        let dx = f64::from(x) + 0.5 - (self.tx.floor() + 0.5);
        let dz = f64::from(z) + 0.5 - (self.tz.floor() + 0.5);
        octagon(dx, dz) <= self.spire_r
    }

    fn floor(&self, x: i32, z: i32) -> Option<i32> {
        if self.spire(x, z) {
            return None;
        }
        let dx = f64::from(x) + 0.5 - self.tx;
        let dz = f64::from(z) + 0.5 - self.tz;
        let d = dx.hypot(dz);
        let edge = self.rim(dz.atan2(dx));
        if d > edge {
            return None;
        }
        let mut y = f64::from(self.ground);
        let inner = (edge - self.ramp).max(self.crater_r + 1.0);
        let k = (d / inner).min(1.0);
        y -= self.driven * (1.0 - k.powf(1.6));
        y -= self.fissure(dx, dz, d) * self.fissure_depth;
        if d < self.crater_r {
            let s = d / self.crater_r;
            y -= self.crater_depth * (1.0 - s * s) * (1.0 - s * s * 0.35);
        }
        let rise = d - (edge - self.ramp);
        if rise > 0.0 {
            y += rise * rise / self.ramp * 1.1;
        }
        Some((y.floor() as i32).max(self.min_y + 2))
    }

    fn fissure(&self, dx: f64, dz: f64, d: f64) -> f64 {
        if d < self.crater_r * 0.8 || d > self.radius * 0.92 {
            return 0.0;
        }
        let a = dz.atan2(dx);
        let mut best: f64 = 0.0;
        for i in 0..14 {
            let ai = f64::from(i) / 14.0 * std::f64::consts::TAU + self.rim_phase[i as usize % 3] * 0.3;
            let wander = (d / self.scale * 0.03 + f64::from(i) * 1.7).sin() * 0.05 + (d / self.scale * 0.011 + f64::from(i)).sin() * 0.04;
            let off = ieee_remainder(a - ai - wander, std::f64::consts::TAU).abs() * d;
            let w = 3.2 * self.scale.max(0.5) * (1.0 - d / self.radius * 0.7);
            if off < w {
                best = best.max(1.0 - off / w);
            }
        }
        best
    }

    fn scorched(&self, x: i32, z: i32) -> Option<&'static str> {
        let d = (f64::from(x) + 0.5 - self.tx).hypot(f64::from(z) + 0.5 - self.tz);
        let f = self.fissure(f64::from(x) + 0.5 - self.tx, f64::from(z) + 0.5 - self.tz, d);
        if f > 0.35 {
            return Some(if f > 0.7 || sk::hash(self.seed, i64::from(x) * 3, i64::from(z) * 5) >= 0.5 {
                "minecraft:magma_block"
            } else {
                "minecraft:blackstone"
            });
        }
        let roll = sk::hash(self.seed, i64::from(x), i64::from(z));
        let jitter = (sk::hash(self.seed, i64::from(z) * 7 + 3, i64::from(x) * 13 - 5) - 0.5) * 24.0 * self.scale;
        let r = (d + jitter) * (200.0 / self.radius);
        let pick = |table: &[(f64, &'static str)], last: Option<&'static str>| {
            table.iter().find(|(limit, _)| roll < *limit).map(|(_, block)| *block).or(last)
        };
        if r < 10.0 {
            pick(&[(0.5, "minecraft:magma_block"), (0.8, "minecraft:obsidian")], Some("minecraft:blackstone"))
        } else if r < self.crater_r * 200.0 / self.radius {
            pick(
                &[
                    (0.34, "minecraft:blackstone"),
                    (0.58, "minecraft:basalt"),
                    (0.7, "minecraft:magma_block"),
                    (0.8, "minecraft:obsidian"),
                ],
                Some("minecraft:cobbled_deepslate"),
            )
        } else if r < 100.0 {
            pick(
                &[
                    (0.3, "minecraft:coarse_dirt"),
                    (0.45, "minecraft:blackstone"),
                    (0.65, "minecraft:gravel"),
                    (0.73, "minecraft:basalt"),
                    (0.86, "minecraft:tuff"),
                    (0.88, "minecraft:magma_block"),
                ],
                Some("minecraft:cobbled_deepslate"),
            )
        } else if r < 170.0 {
            pick(&[(0.35, "minecraft:coarse_dirt"), (0.5, "minecraft:gravel"), (0.7, "minecraft:dirt")], None)
        } else {
            pick(&[(0.4, "minecraft:coarse_dirt")], None)
        }
    }

    /// The tungsten spire, from the world's floor to its ceiling, with its
    /// four fins at the top.
    fn spire_blocks(&self, max_y: i32) -> HashMap<[i32; 3], &'static str> {
        let mut out = HashMap::new();
        let cx = self.tx.floor() as i32;
        let cz = self.tz.floor() as i32;
        let reach = self.fin_span.ceil() as i32 + 1;
        for y in self.min_y..=max_y {
            let from_top = max_y - y;
            let band_at = (y - self.ground).rem_euclid(24);
            let band = band_at == 0 || (band_at == 1 && y > self.ground);
            for dx in -reach..=reach {
                for dz in -reach..=reach {
                    let o = octagon(f64::from(dx), f64::from(dz));
                    let at = [cx + dx, y, cz + dz];
                    if o <= self.spire_r {
                        let shell = o > self.spire_r - 1.0;
                        let stripe = shell && dx.abs() == dz.abs() && dx != 0;
                        let cap = from_top == 0 && o <= self.spire_r * 0.5;
                        out.insert(at, if (!shell || (!stripe && !band)) && !cap { HULL } else { SEAM });
                    } else if from_top < self.fin_height && (dx == 0 || dz == 0) {
                        let k = 1.0 - f64::from(from_top) / f64::from(self.fin_height);
                        let span = self.spire_r + (self.fin_span - self.spire_r) * k.powf(0.7);
                        let r = f64::from(dx.abs() + dz.abs());
                        if r <= span {
                            out.insert(at, if r <= span - 1.0 && from_top != 0 { HULL } else { SEAM });
                        }
                    }
                }
            }
        }
        out
    }
}

fn octagon(dx: f64, dz: f64) -> f64 {
    let ax = dx.abs();
    let az = dz.abs();
    ax.max(az).max((ax + az) * (7071.0 / 10000.0))
}

/// Java's `Math.IEEEremainder`.
fn ieee_remainder(x: f64, y: f64) -> f64 {
    x - (x / y).round_ties_even() * y
}

fn gungnir_tick(game: &mut super::Game<'_>, spell: &mut Spell, t: i32) {
    let Kind::Gungnir(shape) = &spell.kind else {
        return;
    };
    let radius = shape.radius;
    let scale = shape.scale;
    let fin_span = shape.fin_span;
    let crater_floor = f64::from(shape.ground) - shape.crater_depth - 8.0;
    let dim = spell.dimension.clone();
    if t == 322 {
        let gungnir = GungnirShape::new(spell.target, spell.seed, spell.tune.power, spell.min_y);
        let spire = gungnir.spire_blocks(spell.max_y);
        let mut carving = Carving::new(
            spell.owner(),
            Shape::Gungnir(Box::new(gungnir)),
            spell.target,
            radius + 12.0,
            false,
            36,
            None,
            spell.min_y,
            spell.max_y,
        );
        carving.tick(game, &dim, fin_span + 2.0);
        spell.carvings.push(carving);
        paint(game, &dim, &spire);
    }
    let front = sk::gungnir_front(f64::from(t), radius);
    for carving in spell.carvings.iter_mut() {
        if !carving.done() {
            carving.tick(game, &dim, (front - 10.0 * scale).min(radius + 12.0).max(0.0));
        }
    }
    let (cx, cz) = (spell.target[0], spell.target[2]);
    if (322..=370).contains(&t) && (t - 322) % 2 == 0 {
        let reach = front.min(radius + 4.0 * scale);
        if reach > 0.5 {
            let loose = t == 322 || (t - 322) % 10 == 0;
            strike(
                game,
                spell,
                |p| (p[0] - cx).hypot(p[2] - cz) <= reach + 0.3 && p[1] > crater_floor,
                loose.then_some((cx, cz, reach)),
                None,
            );
        }
    }
    let before = sk::gungnir_front(f64::from(t - 1), radius);
    if t > 322 && front > radius && before < radius * 2.5 {
        blast(game, spell, before.max(radius), front.min(radius * 2.5), radius);
    }
}

/// The shock wave past the rim: pushed outward and hurt, harder the closer.
/// Players' own windows push them (`shooting_star::shockwave`).
fn blast(game: &mut super::Game<'_>, spell: &mut Spell, from: f64, to: f64, radius: f64) {
    let (cx, cz) = (spell.target[0], spell.target[2]);
    let dim = spell.dimension.clone();
    for mob in game.native_mobs(&dim) {
        let d = (mob.pos[0] - cx).hypot(mob.pos[2] - cz);
        if d <= from || d > to {
            continue;
        }
        let k = 1.0 - (d - radius) / (radius * 1.5);
        let out = [(mob.pos[0] - cx) / d.max(1e-6), (mob.pos[2] - cz) / d.max(1e-6)];
        let push = 1.2 + 2.8 * k;
        game.native_push_mob(&dim, mob.id, [out[0] * push, 0.6 + 1.2 * k, out[1] * push]);
        let damage = ((4.0 + 26.0 * k * k) as f32).min(spell.tune.damage);
        hit_mob(game, spell, (mob.id, mob.health), damage, false);
    }
    for player in game.native_server_players() {
        if player.dimension != dim || player.game_mode == 1 || player.game_mode == 3 || player.health <= 0.0 {
            continue;
        }
        let d = (player.position[0] - cx).hypot(player.position[2] - cz);
        if d <= from || d > to {
            continue;
        }
        let k = 1.0 - (d - radius) / (radius * 1.5);
        let damage = ((4.0 + 26.0 * k * k) as f32).min(spell.tune.damage);
        hit_player(game, spell, &player, damage, false);
    }
}

// ---- SS-04 Seven Stars ------------------------------------------------------------------------------------------

struct SevenState {
    scale: f64,
    trench_w: f64,
    trench_depth: f64,
    margin: f64,
    nodes: [[f64; 2]; 7],
    ground: [i32; 7],
    /// Indexes into the spell's carvings.
    craters: [Option<usize>; 7],
    links: [Option<usize>; 7],
    /// The star cores, painted again wherever a later cut reaches them.
    cores: HashMap<[i32; 3], &'static str>,
    held: Vec<[i32; 2]>,
}

impl SevenState {
    fn new(origin: [f64; 3], target: [f64; 3], power: f32) -> SevenState {
        let scale = f64::from(power);
        let turn = sk::dipper_turn(origin[0], origin[2], target[0], target[2]);
        let mut nodes = [[0.0; 2]; 7];
        for (i, node) in nodes.iter_mut().enumerate() {
            let n = sk::dipper_node(i, turn, scale);
            *node = [target[0] + n[0], target[2] + n[1]];
        }
        SevenState {
            scale,
            trench_w: 5.5 * scale,
            trench_depth: 12.0 * scale,
            margin: 2.0 * scale,
            nodes,
            ground: [target[1].floor() as i32 - 1; 7],
            craters: [None; 7],
            links: [None; 7],
            cores: HashMap::new(),
            held: Vec::new(),
        }
    }

    fn against_link(&self, k: usize, x: f64, z: f64) -> (f64, f64) {
        let a = self.nodes[sk::DIPPER_LINKS[k][0]];
        let b = self.nodes[sk::DIPPER_LINKS[k][1]];
        let ex = b[0] - a[0];
        let ez = b[1] - a[1];
        let u = (((x - a[0]) * ex + (z - a[1]) * ez) / (ex * ex + ez * ez)).clamp(0.0, 1.0);
        ((x - a[0] - ex * u).hypot(z - a[1] - ez * u), u)
    }
}

fn seven_tick(game: &mut super::Game<'_>, spell: &mut Spell, caster: Option<&super::NativeServerPlayer>, t: i32) {
    let dim = spell.dimension.clone();
    if t == 0 {
        // The stars' own chunks, so the land is there when they fall.
        let owner = spell.owner();
        if let Kind::Seven(state) = &mut spell.kind {
            for node in state.nodes {
                let chunk = [(node[0].floor() as i32) >> 4, (node[1].floor() as i32) >> 4];
                if hold(game, &owner, chunk) {
                    state.held.push(chunk);
                }
            }
        }
    }
    if t == 289 {
        spell.spared = caster.filter(|c| c.game_mode == 1).map(|c| {
            [c.position[0].floor() as i32, c.position[1].floor() as i32 - 1, c.position[2].floor() as i32]
        });
    }
    for i in 0..7 {
        if t == sk::dipper_land_tick(i) {
            seven_land(game, spell, i);
        }
    }
    for k in 0..7 {
        if t == sk::dipper_link_tick(k) {
            seven_link(spell, k);
        }
    }
    // The cuts go on, and the cores stay.
    let mut cut = Vec::new();
    let link_index: Vec<(usize, Option<usize>)> = match &spell.kind {
        Kind::Seven(state) => state.links.iter().copied().enumerate().collect(),
        _ => Vec::new(),
    };
    let link_slots: HashMap<usize, usize> = link_index.iter().filter_map(|(k, c)| c.map(|c| (c, *k))).collect();
    let (trench_w, link_half): (f64, Vec<f64>) = match &spell.kind {
        Kind::Seven(state) => (
            state.trench_w,
            sk::DIPPER_LINKS
                .iter()
                .map(|l| {
                    let a = state.nodes[l[0]];
                    let b = state.nodes[l[1]];
                    (b[0] - a[0]).hypot(b[1] - a[1]) * 0.5
                })
                .collect(),
        ),
        _ => (0.0, Vec::new()),
    };
    for (index, carving) in spell.carvings.iter_mut().enumerate() {
        if carving.done() {
            continue;
        }
        let front = match link_slots.get(&index) {
            Some(&k) => {
                let since = t - sk::dipper_link_tick(k);
                if since >= 8 {
                    f64::MAX
                } else {
                    (link_half[k] + trench_w) * f64::from(since + 1) / 8.0
                }
            }
            None => f64::MAX,
        };
        cut.extend(carving.tick(game, &dim, front));
    }
    repaint_cores(game, spell, &cut);
    for k in 0..7 {
        let at = sk::dipper_link_tick(k);
        if t >= at && t <= at + 10 && (t - at) % 2 == 0 {
            seven_burn_link(game, spell, k);
        }
    }
    if t == spell.skill.duration()
        && let Kind::Seven(state) = &mut spell.kind
    {
        let owner = format!("shooting_star_demo:spell/{}", spell.id);
        for chunk in state.held.drain(..) {
            release(game, &owner, chunk);
        }
    }
}

fn seven_land(game: &mut super::Game<'_>, spell: &mut Spell, i: usize) {
    let dim = spell.dimension.clone();
    let owner = spell.owner();
    let spared = spell.spared;
    let (min_y, max_y) = (spell.min_y, spell.max_y);
    let seed = spell.seed;
    let Kind::Seven(state) = &mut spell.kind else {
        return;
    };
    let node = state.nodes[i];
    let x = node[0].floor() as i32;
    let z = node[1].floor() as i32;
    if let Ok(heights) = game.native_surface_heights(&dim, &[(x, z)], NativeHeightmap::MotionBlockingNoLeaves)
        && let Some(Some(top)) = heights.first()
    {
        state.ground[i] = top - 1;
    }
    let scale = state.scale;
    let r = sk::dipper_crater_radius(i, scale);
    let depth = sk::dipper_crater_depth(i, scale);
    let shape = Shape::Crater {
        i,
        nx: node[0],
        nz: node[1],
        r,
        depth,
        ground: state.ground[i],
        seed,
        min_y,
    };
    let reach = r * 1.1 + state.margin;
    let core = seven_core(state, i, seed, min_y);
    let margin = state.margin;
    let ground = state.ground[i];
    let mut carving = Carving::new(owner, shape, [node[0], 0.0, node[1]], reach, false, 24, spared, min_y, max_y);
    carving.tick(game, &dim, f64::MAX);
    carving.tick(game, &dim, f64::MAX);
    spell.carvings.push(carving);
    let index = spell.carvings.len() - 1;
    if let Kind::Seven(state) = &mut spell.kind {
        state.craters[i] = Some(index);
        for (at, block) in &core {
            state.cores.insert(*at, block);
        }
    }
    paint(game, &dim, &core);
    let below = f64::from(ground) - depth - 4.0 * margin;
    let (nx, nz) = (node[0], node[1]);
    let kill = r + 4.0 * margin;
    strike(
        game,
        spell,
        |p| (p[0] - nx).hypot(p[2] - nz) <= kill && p[1] >= below,
        Some((nx, nz, kill)),
        None,
    );
}

/// Star `i`'s crystal: a central spike and five to seven leaning ones.
fn seven_core(state: &SevenState, i: usize, seed: u64, min_y: i32) -> HashMap<[i32; 3], &'static str> {
    let mut out = HashMap::new();
    let cx = state.nodes[i][0].floor() as i32;
    let cz = state.nodes[i][1].floor() as i32;
    let base = ((f64::from(state.ground[i]) - sk::dipper_crater_depth(i, state.scale)).floor() as i32).max(min_y);
    let size = sk::dipper_crater_radius(i, state.scale) / (36.0 * state.scale);
    let spikes = 5 + (sk::hash(seed, i as i64, 17) * 3.0) as i32;
    for s in -1..spikes {
        let yaw = if s < 0 {
            0.0
        } else {
            (f64::from(s) + sk::hash(seed, i as i64 * 13 + i64::from(s), 5) * 0.6) / f64::from(spikes) * std::f64::consts::TAU
        };
        let lean = if s < 0 { 0.0 } else { 0.35 + sk::hash(seed, i64::from(s), i as i64 * 7) * 0.5 };
        let length = (if s < 0 { 16.0 } else { 7.0 + sk::hash(seed, i as i64 + i64::from(s) * 3, 9) * 7.0 }) * (0.6 + 0.6 * size) * state.scale;
        let thick = (if s < 0 { 2.2 } else { 1.2 }) * state.scale;
        let mut l = 0.0;
        while l < length {
            let w = thick * (1.0 - l / length * 0.85);
            let px = f64::from(cx) + 0.5 + yaw.cos() * lean.sin() * l;
            let py = f64::from(base) + lean.cos() * l;
            let pz = f64::from(cz) + 0.5 + yaw.sin() * lean.sin() * l;
            let r = w.ceil() as i32;
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        if f64::from(dx * dx + dy * dy + dz * dz) <= w * w {
                            let at = [
                                (px + f64::from(dx)).floor() as i32,
                                (py + f64::from(dy)).floor() as i32,
                                (pz + f64::from(dz)).floor() as i32,
                            ];
                            out.insert(at, STAR_CORE);
                        }
                    }
                }
            }
            l += 1.0;
        }
    }
    out
}

fn seven_link(spell: &mut Spell, k: usize) {
    let owner = spell.owner();
    let spared = spell.spared;
    let (min_y, max_y, seed) = (spell.min_y, spell.max_y, spell.seed);
    let Kind::Seven(state) = &spell.kind else {
        return;
    };
    let [a_i, b_i] = sk::DIPPER_LINKS[k];
    let a = state.nodes[a_i];
    let b = state.nodes[b_i];
    let mid = [(a[0] + b[0]) * 0.5, 0.0, (a[1] + b[1]) * 0.5];
    let half = (b[0] - a[0]).hypot(b[1] - a[1]) * 0.5;
    let shape = Shape::Link {
        a,
        b,
        ga: state.ground[a_i],
        gb: state.ground[b_i],
        w: state.trench_w,
        depth: state.trench_depth,
        scale: state.scale,
        k,
        seed,
        min_y,
    };
    let reach = half + state.trench_w + state.margin;
    let carving = Carving::new(owner, shape, mid, reach, false, 24, spared, min_y, max_y);
    spell.carvings.push(carving);
    let index = spell.carvings.len() - 1;
    if let Kind::Seven(state) = &mut spell.kind {
        state.links[k] = Some(index);
    }
}

fn seven_burn_link(game: &mut super::Game<'_>, spell: &mut Spell, k: usize) {
    let (reach, a, b) = match &spell.kind {
        Kind::Seven(state) => (
            state.trench_w + state.margin * 0.75,
            state.nodes[sk::DIPPER_LINKS[k][0]],
            state.nodes[sk::DIPPER_LINKS[k][1]],
        ),
        _ => return,
    };
    let ex = b[0] - a[0];
    let ez = b[1] - a[1];
    let inside = move |p: [f64; 3]| {
        let u = (((p[0] - a[0]) * ex + (p[2] - a[1]) * ez) / (ex * ex + ez * ez)).clamp(0.0, 1.0);
        (p[0] - a[0] - ex * u).hypot(p[2] - a[1] - ez * u) <= reach
    };
    strike(game, spell, inside, None, None);
}

/// Put back any star core a later cut reached, as `keepsCores` does.
fn repaint_cores(game: &mut super::Game<'_>, spell: &Spell, cut: &[[i32; 2]]) {
    let Kind::Seven(state) = &spell.kind else {
        return;
    };
    if cut.is_empty() || state.cores.is_empty() {
        return;
    }
    let chunks: HashSet<[i32; 2]> = cut.iter().copied().collect();
    let again: HashMap<[i32; 3], &'static str> = state
        .cores
        .iter()
        .filter(|(at, _)| chunks.contains(&[at[0] >> 4, at[2] >> 4]))
        .map(|(at, block)| (*at, *block))
        .collect();
    if !again.is_empty() {
        paint(game, &spell.dimension, &again);
    }
}

// ---- SS-05 Tesseract --------------------------------------------------------------------------------------------

struct TessState {
    scale: f64,
    zone: i32,
    cell: i32,
    ox: i32,
    oz: i32,
    /// The cubes' bottom: a cell's height under the target.
    bottom: i32,
    jobs: std::collections::VecDeque<(i32, i32)>,
    current: Option<Cube>,
    lifted: usize,
}

/// One cell on its way up: its chunks held, then read, then lifted.
struct Cube {
    i: i32,
    j: i32,
    held: Vec<[i32; 2]>,
    columns: Vec<(i32, i32)>,
    /// Per column: the runs `(y, height, block)` of what stood there.
    runs: Vec<Vec<(i32, u32, String)>>,
    read: usize,
    waited: i32,
}

impl TessState {
    fn new(target: [f64; 3], power: f32, min_y: i32) -> TessState {
        let scale = sk::tess_scale(power);
        let zone = sk::tess_zone(scale);
        let cell = zone / 8;
        let ground = target[1].floor() as i32;
        TessState {
            scale,
            zone,
            cell,
            ox: sk::tess_origin(target[0], zone),
            oz: sk::tess_origin(target[2], zone),
            bottom: (ground - 1 - cell).max(min_y + 1),
            jobs: std::collections::VecDeque::new(),
            current: None,
            lifted: 0,
        }
    }

    fn release(&mut self, game: &mut super::Game<'_>, owner: &str) {
        if let Some(cube) = self.current.take() {
            for chunk in cube.held {
                release(game, owner, chunk);
            }
        }
    }
}

fn tess_tick(game: &mut super::Game<'_>, spell: &mut Spell, caster: Option<&super::NativeServerPlayer>, t: i32) {
    if t == 31
        && let Some(c) = caster
        && let Kind::Tess(state) = &spell.kind
    {
        let zone = f64::from(state.zone) * 0.72;
        let ticks = spell.real_ticks(600);
        spell.spared = evacuate(game, spell, c, zone, ticks);
    }
    // Each cell on its own tick, ring by ring from the middle, as the window lights it.
    if let Kind::Tess(state) = &mut spell.kind {
        for (index, at) in super::shooting_star_tesseract::cube_ticks().iter().enumerate() {
            if *at == t {
                state.jobs.push_back((index as i32 % 8, index as i32 / 8));
            }
        }
    }
    tess_work(game, spell);
}

/// One step of the cubes' work a tick: hold a cell's chunks, read it in
/// pieces, then lift it and floor its pit.
fn tess_work(game: &mut super::Game<'_>, spell: &mut Spell) {
    let owner = spell.owner();
    let dim = spell.dimension.clone();
    let (min_y, max_y, seed) = (spell.min_y, spell.max_y, spell.seed);
    let Kind::Tess(state) = &mut spell.kind else {
        return;
    };
    if state.current.is_none() {
        let Some((i, j)) = state.jobs.pop_front() else {
            return;
        };
        let x0 = state.ox + i * state.cell;
        let z0 = state.oz + j * state.cell;
        let mut held = Vec::new();
        for cx in (x0 >> 4)..=((x0 + state.cell - 1) >> 4) {
            for cz in (z0 >> 4)..=((z0 + state.cell - 1) >> 4) {
                if hold(game, &owner, [cx, cz]) {
                    held.push([cx, cz]);
                }
            }
        }
        let mut columns = Vec::with_capacity((state.cell * state.cell) as usize);
        for x in x0..x0 + state.cell {
            for z in z0..z0 + state.cell {
                columns.push((x, z));
            }
        }
        let n = columns.len();
        state.current = Some(Cube {
            i,
            j,
            held,
            columns,
            runs: vec![Vec::new(); n],
            read: 0,
            waited: 0,
        });
        return;
    }
    let Some(cube) = state.current.as_mut() else {
        return;
    };
    let level_height = (max_y - min_y + 1) as u32;
    let per_read = (1_048_576 / level_height.max(1)) as usize;
    if cube.read < cube.columns.len() {
        let end = (cube.read + per_read).min(cube.columns.len());
        let part = &cube.columns[cube.read..end];
        // Wait until the cell's land is loaded (a few seconds at most).
        let loaded = game
            .native_surface_heights(&dim, part, NativeHeightmap::WorldSurface)
            .map(|h| h.iter().all(Option::is_some))
            .unwrap_or(false);
        if !loaded && cube.waited < 200 {
            cube.waited += 1;
            return;
        }
        match game.native_read_columns(&dim, part, NativeHeightmap::WorldSurface, (level_height - 1, 1)) {
            Ok(read) => {
                for (k, base) in read.bases.iter().enumerate() {
                    let Some(base) = base else { continue };
                    let y0 = base - (level_height as i32 - 1);
                    let column = &read.blocks[k * level_height as usize..(k + 1) * level_height as usize];
                    let mut runs: Vec<(i32, u32, String)> = Vec::new();
                    for y in state.bottom..*base {
                        let index = y - y0;
                        if index < 0 || index as usize >= column.len() {
                            continue;
                        }
                        let name = &read.palette[column[index as usize] as usize];
                        if is_air(name) {
                            continue;
                        }
                        match runs.last_mut() {
                            Some((start, height, block)) if *start + *height as i32 == y && block == name => *height += 1,
                            _ => runs.push((y, 1, name.clone())),
                        }
                    }
                    cube.runs[cube.read + k] = runs;
                }
            }
            Err(error) => log::warn!("[{ID}] a Tesseract cell could not be read: {error}"),
        }
        cube.read = end;
        return;
    }
    // Lift: the pit cut and floored, then the cube set up in the sky.
    let Some(cube) = state.current.take() else {
        return;
    };
    let ring = sk::tess_ring(cube.i, cube.j);
    let cell = state.cell;
    let bottom = state.bottom;
    let room = max_y - 2 - bottom - cell;
    let least = cell + (40.0 * state.scale).round() as i32;
    let lift = if room >= least {
        sk::tess_lift(seed, cube.i, cube.j, state.scale).min(room).max(least)
    } else if room >= 12 {
        room
    } else {
        12
    };
    let depth = (4 - ring) * (20.0 * state.scale).round() as i32;
    let floor = (bottom - depth).max(bottom.min(min_y + 4));
    let hue = format!("shooting_star_demo:lattice_edge[hue={}]", (ring * 2 + cube.i + cube.j).rem_euclid(7));
    let mut edit = Edit::default();
    for (k, (x, z)) in cube.columns.iter().enumerate() {
        if floor + 1 <= max_y {
            edit.column(*x, *z, floor + 1, (max_y - floor) as u32, AIR);
        }
        edit.column(*x, *z, floor, 1, "minecraft:bedrock");
        for (y, height, block) in &cube.runs[k] {
            edit.column(*x, *z, y + lift, *height, block);
        }
    }
    // The cage's twelve edges round the risen cube.
    let x0 = state.ox + cube.i * cell;
    let z0 = state.oz + cube.j * cell;
    let (y0, y1) = (bottom + lift, (bottom + lift + cell - 1).min(max_y));
    for x in x0..x0 + cell {
        for z in z0..z0 + cell {
            let on_x = x == x0 || x == x0 + cell - 1;
            let on_z = z == z0 || z == z0 + cell - 1;
            if on_x && on_z {
                edit.column(x, z, y0, (y1 - y0 + 1).max(1) as u32, &hue);
            } else if on_x || on_z {
                edit.column(x, z, y0, 1, &hue);
                edit.column(x, z, y1, 1, &hue);
            }
        }
    }
    apply(game, &dim, &edit);
    for chunk in &cube.held {
        release(game, &owner, *chunk);
    }
    state.lifted += 1;
    let lifted = state.lifted;
    // What stood on the cell rises with it.
    let (fx0, fz0, fx1, fz1) = (f64::from(x0), f64::from(z0), f64::from(x0 + cell), f64::from(z0 + cell));
    let inside = |p: [f64; 3]| p[0] >= fx0 && p[0] < fx1 && p[2] >= fz0 && p[2] < fz1 && p[1] >= f64::from(bottom);
    for mob in game.native_mobs(&dim) {
        if inside(mob.pos) {
            game.native_teleport_mob(&dim, mob.id, [mob.pos[0], mob.pos[1] + f64::from(lift), mob.pos[2]]);
        }
    }
    let mut moves = Vec::new();
    for player in game.native_server_players() {
        if player.dimension == dim && inside(player.position) {
            moves.push(format!(
                "execute in {dim} run tp {} {:.2} {:.2} {:.2}",
                player.name,
                player.position[0],
                player.position[1] + f64::from(lift),
                player.position[2]
            ));
        }
    }
    for line in moves {
        game.native_run_command(&line);
    }
    if lifted % 8 == 0 || lifted == 64 {
        let caster = spell.caster;
        deny(game, caster, format!("{} — raising the land: {lifted} of 64 cubes up", spell.skill.title()));
    }
}

fn is_air(name: &str) -> bool {
    let bare = name.split('[').next().unwrap_or(name);
    matches!(bare, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
}

// ---- SS-06 Gravitia ---------------------------------------------------------------------------------------------

fn grav_tick(game: &mut super::Game<'_>, spell: &mut Spell, t: i32) {
    if t == 1 {
        // Gravitia is cast under the night sky, as the original sets it.
        game.native_run_command(&format!("execute in {} run time set midnight", spell.dimension));
    }
    if t == spell.skill.impact() {
        city::banish(game, &spell.dimension, spell.seed as u32);
        log::info!("[{ID}] Gravitia: {} is sent into Imaginary Space", spell.dimension);
    }
}

// ---- SS-H1 Event Horizon (the port's own) -----------------------------------------------------------------------

/// The hole's sphere as it is eaten: its centre, the ground it found there
/// (read before anything was eaten), and how far it has got.
struct HoleState {
    centre: [f64; 3],
    radius: f64,
    /// Each column's ground (one over its top block) round the centre, read at the start.
    ground: HashMap<(i32, i32), i32>,
    /// The radius eaten so far.
    eaten: f64,
    rim: bool,
}

/// The rim's blocks: fused obsidian, some still glowing.
const RIM: [(f64, &str); 4] = [
    (0.5, "minecraft:obsidian"),
    (0.66, "minecraft:crying_obsidian"),
    (0.86, "minecraft:magma_block"),
    (1.0, "minecraft:blackstone"),
];

impl HoleState {
    fn new(target: [f64; 3], power: f32) -> HoleState {
        HoleState { centre: sk::eh_center(target, power), radius: sk::eh_radius(power), ground: HashMap::new(), eaten: 0.0, rim: false }
    }

    fn reach(&self) -> i32 {
        (self.radius + 3.0).ceil() as i32
    }

    /// Half the height of the sphere of radius `r` over column `x, z` (None: outside it).
    fn half(&self, r: f64, x: i32, z: i32) -> Option<f64> {
        let dx = f64::from(x) + 0.5 - self.centre[0];
        let dz = f64::from(z) + 0.5 - self.centre[2];
        let h = r * r - dx * dx - dz * dz;
        (h > 0.0).then(|| h.sqrt())
    }
}

fn hole_tick(game: &mut super::Game<'_>, spell: &mut Spell, caster: Option<&super::NativeServerPlayer>, t: i32) {
    let dim = spell.dimension.clone();
    let Kind::Hole(state) = &mut spell.kind else { return };
    let (centre, radius, reach) = (state.centre, state.radius, state.reach());
    if t == 1 {
        // Its land stays loaded, and its ground is read before anything is eaten.
        let owner = spell.owner();
        let (cx, cz) = ((centre[0].floor() as i32) >> 4, (centre[2].floor() as i32) >> 4);
        let chunks = (reach >> 4) + 1;
        for dx in -chunks..=chunks {
            for dz in -chunks..=chunks {
                let chunk = [cx + dx, cz + dz];
                if !spell.held.contains(&chunk) && hold(game, &owner, chunk) {
                    spell.held.push(chunk);
                }
            }
        }
        let Kind::Hole(state) = &mut spell.kind else { return };
        let mut columns = Vec::new();
        for x in -reach..=reach {
            for z in -reach..=reach {
                columns.push((centre[0].floor() as i32 + x, centre[2].floor() as i32 + z));
            }
        }
        if let Ok(heights) = game.native_surface_heights(&dim, &columns, NativeHeightmap::MotionBlockingNoLeaves) {
            for (column, top) in columns.into_iter().zip(heights) {
                if let Some(top) = top {
                    state.ground.insert(column, top);
                }
            }
        }
    }
    if t == 31
        && let Some(c) = caster
    {
        let ticks = spell.real_ticks(sk::EH_END - 31);
        spell.spared = evacuate(game, spell, c, radius + 4.0, ticks);
    }
    let Kind::Hole(state) = &mut spell.kind else { return };

    // The sphere eaten outward, a shell every other tick.
    if (sk::EH_FEED..=sk::EH_FULL).contains(&t) && (t % 2 == 0 || t == sk::EH_FULL) {
        let r = radius * sk::eh_eaten(f64::from(t));
        if r > state.eaten + 0.25 {
            let mut edit = Edit::default();
            let (cx, cz) = (centre[0].floor() as i32, centre[2].floor() as i32);
            let ri = r.ceil() as i32;
            for x in cx - ri..=cx + ri {
                for z in cz - ri..=cz + ri {
                    let Some(h) = state.half(r, x, z) else { continue };
                    if spell.spared.is_some_and(|p| (p[0] - x).abs() <= 2 && (p[2] - z).abs() <= 2) {
                        continue;
                    }
                    let top = (centre[1] + h).floor() as i32;
                    let low = ((centre[1] - h).ceil() as i32).max(spell.min_y + 1);
                    match state.half(state.eaten, x, z) {
                        None => edit.column(x, z, low, (top - low + 1).max(0) as u32, AIR),
                        Some(old) => {
                            // Only the new shell: over the old cap and under the old bowl.
                            let old_top = (centre[1] + old).floor() as i32;
                            let old_low = (centre[1] - old).ceil() as i32;
                            edit.column(x, z, old_top + 1, (top - old_top).max(0) as u32, AIR);
                            edit.column(x, z, low, (old_low - low).max(0) as u32, AIR);
                        }
                    }
                }
            }
            apply(game, &dim, &edit);
            let Kind::Hole(state) = &mut spell.kind else { return };
            state.eaten = r;
        }
    }

    // The land torn into orbit: blocks of the ground about to be eaten fly
    // up and spiral in; whatever reaches the hole is gone.
    if (sk::EH_FEED - 10..sk::EH_FULL).contains(&t) {
        let Kind::Hole(state) = &spell.kind else { return };
        let r = radius * sk::eh_eaten(f64::from(t) + 12.0);
        let count = (2.0 + radius / 12.0) as u64;
        let mut picks = Vec::new();
        for k in 0..count {
            let a = sk::hash(spell.seed, i64::from(t), k as i64) * std::f64::consts::TAU;
            let d = r * (0.75 + 0.3 * sk::hash(spell.seed, k as i64, i64::from(t) * 3 + 1)) + 1.5;
            let x = (centre[0] + a.cos() * d).floor() as i32;
            let z = (centre[2] + a.sin() * d).floor() as i32;
            if let Some(&ground) = state.ground.get(&(x, z))
                && (f64::from(ground) - centre[1]).abs() <= radius
            {
                picks.push((x, z));
            }
        }
        // The top block of each, as the server has it.
        // No debris from land that stays whole.
        let read = if picks.is_empty() || !spell.terrain {
            None
        } else {
            game.native_read_columns(&dim, &picks, NativeHeightmap::MotionBlockingNoLeaves, (1, 0)).ok()
        };
        if let Some(read) = read {
            for (i, (x, z)) in picks.iter().copied().enumerate() {
                let (Some(Some(top)), Some(&index)) = (read.bases.get(i), read.blocks.get(i)) else { continue };
                let Some(name) = read.palette.get(usize::from(index)) else { continue };
                let block = name.split('[').next().unwrap_or("");
                if block.is_empty() || is_air(block) || block.contains("water") || block.contains("lava") {
                    continue;
                }
                // Out and round: the spiral's way, and up off the ground.
                let (ox, oz) = (centre[0] - f64::from(x), centre[2] - f64::from(z));
                let n = ox.hypot(oz).max(0.1);
                let (ix, iz) = (ox / n, oz / n);
                let m = [ix * 0.35 - iz * 0.55, 0.75, iz * 0.35 + ix * 0.55];
                game.native_run_command(&format!(
                    "execute in {dim} run summon minecraft:falling_block {}.5 {} {}.5 {{BlockState:{{Name:\"{block}\"}},Time:1,DropItem:0b,CancelDrop:1b,HurtEntities:0b,Motion:[{:.3},{:.3},{:.3}]}}",
                    x,
                    top + 1,
                    z,
                    m[0],
                    m[1],
                    m[2]
                ));
            }
        }
    }
    if (sk::EH_LAND..sk::EH_FLASH).contains(&t) {
        let pull = radius * 3.0;
        let (x, y, z) = (centre[0], centre[1], centre[2]);
        // The torn blocks swirl in, faster the nearer they get.
        game.native_run_command(&format!(
            "execute in {dim} positioned {x:.2} {y:.2} {z:.2} as @e[type=minecraft:falling_block,distance=..{pull:.0}] at @s facing {x:.2} {y:.2} {z:.2} run tp @s ^0.55 ^0.12 ^0.9"
        ));
        let core = (radius * 0.12).max(1.5);
        game.native_run_command(&format!(
            "execute in {dim} positioned {x:.2} {y:.2} {z:.2} run kill @e[type=minecraft:falling_block,distance=..{core:.1}]"
        ));
        for kind in ["minecraft:item", "minecraft:experience_orb", "minecraft:arrow"] {
            game.native_run_command(&format!(
                "execute in {dim} positioned {x:.2} {y:.2} {z:.2} run kill @e[type={kind},distance=..{:.1}]",
                (state_eaten(spell) + 1.0).max(core)
            ));
        }
        // The living are dragged in, and what the sphere holds is taken.
        let feeding = t >= sk::EH_FEED;
        for mob in game.native_mobs(&dim) {
            let d = [x - mob.pos[0], y - mob.pos[1], z - mob.pos[2]];
            let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if n < pull && n > 0.5 {
                let k = 0.08 + 0.5 * (1.0 - n / pull).powi(2);
                game.native_push_mob(&dim, mob.id, [d[0] / n * k - d[2] / n * k * 0.4, d[1] / n * k + 0.04, d[2] / n * k + d[0] / n * k * 0.4]);
            }
        }
        if feeding {
            let r = state_eaten(spell) + 1.0;
            strike(game, spell, |p| ((p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2)).sqrt() <= r, None, None);
        }
    }

    // The flash: the hole evaporates, the light burns what stood near, the rim fuses.
    if t == sk::EH_FLASH {
        let (x, y, z) = (centre[0], centre[1], centre[2]);
        let burn = radius * 1.5;
        strike(game, spell, |p| ((p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2)).sqrt() <= burn, None, None);
        hole_rim(game, spell);
        game.native_run_command(&format!(
            "execute in {dim} positioned {x:.2} {y:.2} {z:.2} run kill @e[type=minecraft:falling_block,distance=..{:.0}]",
            radius * 3.0
        ));
    }
}

fn state_eaten(spell: &Spell) -> f64 {
    match &spell.kind {
        Kind::Hole(state) => state.eaten,
        _ => 0.0,
    }
}

/// The rim: one to two blocks of the ground round the sphere, fused, where
/// there was ground (never in the air over it).
fn hole_rim(game: &mut super::Game<'_>, spell: &mut Spell) {
    let dim = spell.dimension.clone();
    let seed = spell.seed;
    let min_y = spell.min_y;
    let Kind::Hole(state) = &mut spell.kind else { return };
    if state.rim {
        return;
    }
    state.rim = true;
    let inner = state.radius;
    let outer = state.radius + 1.6;
    let (cx, cz) = (state.centre[0].floor() as i32, state.centre[2].floor() as i32);
    let ro = outer.ceil() as i32;
    let mut blocks: HashMap<[i32; 3], &'static str> = HashMap::new();
    for x in cx - ro..=cx + ro {
        for z in cz - ro..=cz + ro {
            let Some(&ground) = state.ground.get(&(x, z)) else { continue };
            let Some(ho) = state.half(outer, x, z) else { continue };
            let lo = ((state.centre[1] - ho).ceil() as i32).max(min_y + 1);
            let hi = ((state.centre[1] + ho).floor() as i32).min(ground - 1);
            for y in lo..=hi {
                let dy = f64::from(y) + 0.5 - state.centre[1];
                let dx = f64::from(x) + 0.5 - state.centre[0];
                let dz = f64::from(z) + 0.5 - state.centre[2];
                let d = (dx * dx + dy * dy + dz * dz).sqrt();
                if d < inner || d > outer {
                    continue;
                }
                let roll = sk::hash(seed, i64::from(x) * 31 + i64::from(y), i64::from(z) * 17 - i64::from(y));
                let block = RIM.iter().find(|(k, _)| roll < *k).map_or("minecraft:obsidian", |(_, b)| b);
                blocks.insert([x, y, z], block);
            }
        }
    }
    paint(game, &dim, &blocks);
}

// ---- SS-H2 Supernova (the port's own) ---------------------------------------------------------------------------

/// How far the ground front has burnt so far.
struct NovaState {
    burnt: f64,
}

/// A plant, a flower, a vine: what burns away whole.
fn burns_away(block: &str) -> bool {
    let b = block.trim_start_matches("minecraft:");
    matches!(
        b,
        "short_grass" | "tall_grass" | "grass" | "fern" | "large_fern" | "dead_bush" | "bush" | "vine" | "glow_lichen" | "dandelion"
            | "poppy" | "blue_orchid" | "allium" | "azure_bluet" | "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "sunflower"
            | "lilac" | "rose_bush" | "peony" | "sweet_berry_bush" | "pink_petals" | "wildflowers" | "leaf_litter" | "firefly_bush"
            | "short_dry_grass" | "tall_dry_grass" | "torchflower" | "pitcher_plant" | "spore_blossom" | "moss_carpet" | "snow"
            | "brown_mushroom" | "red_mushroom" | "sugar_cane" | "bamboo" | "cactus" | "hanging_roots"
    ) || b.ends_with("_tulip")
        || b.ends_with("_sapling")
        || b.ends_with("_leaves")
}

/// What the front leaves of a column's top block, `k` of the way out (0 the
/// middle, 1 the edge), `roll` a die for it.
fn burnt(block: &str, k: f64, roll: f64) -> Option<&'static str> {
    let b = block.trim_start_matches("minecraft:");
    let soil = matches!(b, "grass_block" | "podzol" | "mycelium" | "moss_block" | "dirt_path" | "farmland" | "rooted_dirt" | "pale_moss_block");
    if k < 0.22 {
        // Under the star: the land fused to glass, threaded with magma.
        return match b {
            "sand" | "gravel" | "suspicious_sand" | "suspicious_gravel" => Some(if roll < 0.8 { "minecraft:glass" } else { "minecraft:tinted_glass" }),
            "red_sand" => Some("minecraft:orange_stained_glass"),
            _ if soil || b == "dirt" || b == "coarse_dirt" || b == "clay" => Some(if roll < 0.42 {
                "minecraft:glass"
            } else if roll < 0.58 {
                "minecraft:tinted_glass"
            } else if roll < 0.75 {
                "minecraft:magma_block"
            } else {
                "minecraft:smooth_basalt"
            }),
            "stone" | "andesite" | "diorite" | "granite" | "deepslate" | "tuff" => (roll < 0.3).then_some("minecraft:magma_block"),
            "snow_block" | "powder_snow" => Some("minecraft:air"),
            "ice" => Some("minecraft:water"),
            _ => None,
        };
    }
    if k < 0.7 {
        return match b {
            _ if soil => Some(if roll < 0.55 {
                "minecraft:coarse_dirt"
            } else if roll < 0.9 {
                "minecraft:dirt"
            } else if roll < 0.96 {
                "minecraft:magma_block"
            } else {
                "minecraft:smooth_basalt"
            }),
            "sand" | "suspicious_sand" => (roll < 0.7).then_some("minecraft:glass"),
            "red_sand" => (roll < 0.6).then_some("minecraft:orange_stained_glass"),
            "snow_block" | "powder_snow" => Some("minecraft:air"),
            "ice" => Some("minecraft:water"),
            _ => None,
        };
    }
    // The edge: singed.
    let singe = 1.0 - (k - 0.7) / 0.3;
    match b {
        "grass_block" | "podzol" | "mycelium" if roll < 0.2 + 0.5 * singe => Some(if roll < 0.15 { "minecraft:coarse_dirt" } else { "minecraft:dirt" }),
        "sand" if roll < 0.25 * singe => Some("minecraft:glass"),
        _ => None,
    }
}

fn nova_tick(game: &mut super::Game<'_>, spell: &mut Spell, t: i32) {
    if t < sk::SN_HIT {
        return;
    }
    let dim = spell.dimension.clone();
    let power = spell.tune.power;
    let radius = sk::sn_radius(power);
    let front = sk::sn_ring(f64::from(t), power);
    let target = spell.target;
    // The living in the front's way: hurt and thrown outward.
    if front > 0.0 && front < radius + 1.0 {
        let (x, y, z) = (target[0], target[1], target[2]);
        let band = |p: [f64; 3]| {
            let d = (p[0] - x).hypot(p[2] - z);
            d <= front + 1.5 && d >= front - 8.0 && (p[1] - y).abs() < 60.0
        };
        strike(game, spell, band, None, None);
        for mob in game.native_mobs(&dim) {
            let (dx, dz) = (mob.pos[0] - x, mob.pos[2] - z);
            let d = dx.hypot(dz);
            if band(mob.pos) && d > 0.5 {
                game.native_push_mob(&dim, mob.id, [dx / d * 1.4, 0.6, dz / d * 1.4]);
            }
        }
    }
    let Kind::Nova(state) = &mut spell.kind else { return };
    if front <= state.burnt + 0.5 && t < sk::SN_END {
        return;
    }
    let from = state.burnt;
    state.burnt = front;
    if !spell.terrain {
        return;
    }
    // The ring the front crossed this tick, read and burnt.
    let (cx, cz) = (target[0], target[2]);
    let ri = front.ceil() as i32 + 1;
    let mut columns = Vec::new();
    for x in (cx.floor() as i32 - ri)..=(cx.floor() as i32 + ri) {
        for z in (cz.floor() as i32 - ri)..=(cz.floor() as i32 + ri) {
            let d = (f64::from(x) + 0.5 - cx).hypot(f64::from(z) + 0.5 - cz);
            if d >= from && d < front {
                columns.push((x, z));
            }
        }
    }
    const DEPTH: u32 = 8;
    let mut edit = Edit::default();
    for batch in columns.chunks(8192) {
        let Ok(read) = game.native_read_columns(&dim, batch, NativeHeightmap::WorldSurface, (DEPTH, 0)) else { continue };
        for (i, &(x, z)) in batch.iter().enumerate() {
            let Some(Some(base)) = read.bases.get(i).copied() else { continue };
            let d = (f64::from(x) + 0.5 - cx).hypot(f64::from(z) + 0.5 - cz);
            let k = d / radius;
            let at = i * DEPTH as usize;
            // From the top down: what burns away goes, the first solid block is burnt.
            for j in (0..DEPTH as usize).rev() {
                let Some(&index) = read.blocks.get(at + j) else { break };
                let Some(name) = read.palette.get(usize::from(index)) else { break };
                let block = name.split('[').next().unwrap_or("");
                let y = base - DEPTH as i32 + j as i32;
                if block.is_empty() || is_air(block) {
                    continue;
                }
                let roll = sk::hash(spell.seed, i64::from(x) * 7 + i64::from(y), i64::from(z) * 13 - i64::from(y));
                if burns_away(block) {
                    let leaves = block.ends_with("_leaves");
                    // At the edge some leaves and grass survive, singed.
                    if !(leaves || k < 0.85 || roll < 0.75) || (leaves && k > 0.7 && roll > 0.55) {
                        continue;
                    }
                    edit.column(x, z, y, 1, AIR);
                    continue;
                }
                if block.contains("water") || block.contains("lava") || block.ends_with("_log") || block.ends_with("_wood") {
                    break;
                }
                if let Some(to) = burnt(block, k, roll) {
                    edit.column(x, z, y, 1, to);
                }
                break;
            }
        }
    }
    apply(game, &dim, &edit);
}

// ---- what a spell does to the living ----------------------------------------------------------------------------

/// Hit every mob and player `inside` takes, and with `loose` sweep the items
/// and orbs out of that column, as the original's `burn` does.
fn strike(
    game: &mut super::Game<'_>,
    spell: &mut Spell,
    inside: impl Fn([f64; 3]) -> bool,
    loose: Option<(f64, f64, f64)>,
    damage: Option<f32>,
) {
    let dim = spell.dimension.clone();
    let damage = damage.unwrap_or(spell.tune.damage);
    for mob in game.native_mobs(&dim) {
        if inside(mob.pos) {
            hit_mob(game, spell, (mob.id, mob.health), damage, true);
        }
    }
    for player in game.native_server_players() {
        if player.dimension != dim || player.game_mode == 1 || player.game_mode == 3 || player.health <= 0.0 {
            continue;
        }
        if inside(player.position) {
            hit_player(game, spell, &player, damage, true);
        }
    }
    if let Some((cx, cz, r)) = loose {
        let h = spell.max_y - spell.min_y + 128;
        for kind in ["minecraft:item", "minecraft:experience_orb", "minecraft:arrow", "minecraft:falling_block"] {
            game.native_run_command(&format!(
                "execute in {dim} run kill @e[type={kind},x={:.1},y={},z={:.1},dx={:.1},dy={h},dz={:.1}]",
                cx - r,
                spell.min_y - 64,
                cz - r,
                2.0 * r,
                2.0 * r
            ));
        }
    }
}

fn hit_mob(game: &mut super::Game<'_>, spell: &mut Spell, mob: (u32, f32), damage: f32, beam: bool) {
    let (id, health) = mob;
    let dim = spell.dimension.clone();
    if damage == f32::INFINITY && beam {
        game.native_kill_mob(&dim, id);
        return;
    }
    // A mob is hurt at most every ten ticks, as vanilla's hurt cooldown.
    if spell.mob_hits.get(&id).is_some_and(|last| spell.age - last < 10) {
        return;
    }
    spell.mob_hits.insert(id, spell.age);
    let left = health - damage;
    if left <= 0.0 {
        game.native_kill_mob(&dim, id);
    } else {
        game.native_set_mob_health(&dim, id, left);
    }
}

fn hit_player(game: &mut super::Game<'_>, spell: &mut Spell, player: &super::NativeServerPlayer, damage: f32, beam: bool) {
    let last = spell.player_hits.get(&player.uuid).copied();
    if last.is_some_and(|last| spell.age - last < 10) {
        return;
    }
    spell.player_hits.insert(player.uuid, spell.age);
    let erases = damage == f32::INFINITY && beam;
    // Credited to the caster while they are here to be named.
    let online = game.native_server_players().iter().any(|p| p.uuid == spell.caster);
    let by = if player.uuid == spell.caster || !online { String::new() } else { format!(" by {}", spell.caster_name) };
    let line = if erases && last.is_some() {
        // Still standing after an erasing hit (a totem): erased for good.
        format!("kill {}", player.name)
    } else if erases {
        format!("damage {} 1000000 shooting_star_demo:shooting_star{by}", player.name)
    } else {
        let kind = if spell.tune.erases() { "shooting_star" } else { "shooting_star_hit" };
        format!("damage {} {:.1} shooting_star_demo:{kind}{by}", player.name, damage.min(1.0e6))
    };
    game.native_run_command(&line);
}

/// The caster stands the spell out: resistance and fire resistance, as
/// `SpellUtil.ward`.
fn ward(game: &mut super::Game<'_>, caster: &super::NativeServerPlayer) {
    game.native_run_command(&format!("effect give {} minecraft:resistance 2 4 true", caster.name));
    game.native_run_command(&format!("effect give {} minecraft:fire_resistance 2 0 true", caster.name));
}

/// A creative caster inside the strike zone is moved clear of it, high up
/// and facing it (`GreaterTeleportation.lift`): the column under them is
/// spared when nowhere is found.
fn evacuate(
    game: &mut super::Game<'_>,
    spell: &Spell,
    caster: &super::NativeServerPlayer,
    zone: f64,
    ticks: i32,
) -> Option<[i32; 3]> {
    if caster.game_mode != 1 && caster.game_mode != 3 {
        return None;
    }
    let target = spell.target;
    if (caster.position[0] - target[0]).hypot(caster.position[2] - target[2]) >= zone {
        return None;
    }
    let dim = spell.dimension.clone();
    let flat = [caster.position[0] - target[0], caster.position[2] - target[2]];
    let len = flat[0].hypot(flat[1]);
    let home = if len >= 1.0 {
        [flat[0] / len, flat[1] / len]
    } else {
        let yaw = f64::from(caster.yaw).to_radians();
        [yaw.sin(), -yaw.cos()]
    };
    let distance = zone + 25.0;
    let (min_y, height) = game.native_world_height();
    let ceiling = f64::from(min_y + height) - 2.0;
    let mut to = None;
    for turn in [0.0_f64, 20.0, -20.0, 40.0, -40.0, 60.0, -60.0] {
        let (s, c) = turn.to_radians().sin_cos();
        let x = target[0] + (home[0] * c - home[1] * s) * distance;
        let z = target[2] + (home[0] * s + home[1] * c) * distance;
        let mut samples = Vec::new();
        for dx in [-8, 0, 8] {
            for dz in [-8, 0, 8] {
                samples.push((x.floor() as i32 + dx, z.floor() as i32 + dz));
            }
        }
        let Ok(heights) = game.native_surface_heights(&dim, &samples, NativeHeightmap::MotionBlocking) else {
            continue;
        };
        if heights[4].is_none() {
            continue;
        }
        let high = heights.iter().flatten().copied().max().unwrap_or(min_y);
        to = Some([x, (f64::from(high) + 48.0).min(ceiling), z]);
        break;
    }
    let Some(to) = to else {
        return Some([caster.position[0].floor() as i32, caster.position[1].floor() as i32 - 1, caster.position[2].floor() as i32]);
    };
    let dx = target[0] - to[0];
    let dz = target[2] - to[2];
    let dy = target[1] - (to[1] + 1.62);
    let yaw = dz.atan2(dx).to_degrees() - 90.0;
    let pitch = -dy.atan2(dx.hypot(dz)).to_degrees();
    let name = caster.name.clone();
    let from = caster.position;
    game.native_run_command(&format!(
        "execute in {dim} run tp {name} {:.2} {:.2} {:.2} {:.1} {:.1}",
        to[0], to[1], to[2], yaw, pitch
    ));
    game.native_run_command(&format!("effect give {name} minecraft:slow_falling {} 0 true", (ticks / 20).max(1) + 5));
    game.native_run_command(&format!("title {name} times 2 30 12"));
    game.native_run_command(&format!("title {name} subtitle {{\"text\":\"EVAC // CLEAR OF THE STRIKE ZONE\",\"color\":\"#FF3248\"}}"));
    game.native_run_command(&format!("title {name} title \"\""));
    for at in [from, to] {
        game.native_level_play_sound("shooting_star_demo:star_evac", at, 7, 1.2, 1.0);
    }
    None
}

// ---- cutting the land -------------------------------------------------------------------------------------------

/// The ground a carving leaves: per column, the highest block kept (`None`
/// leaves the column alone) and what the new surface is made of.
enum Shape {
    Cylinder { cx: f64, cz: f64, r: f64 },
    Gungnir(Box<GungnirShape>),
    Crater { i: usize, nx: f64, nz: f64, r: f64, depth: f64, ground: i32, seed: u64, min_y: i32 },
    Link { a: [f64; 2], b: [f64; 2], ga: i32, gb: i32, w: f64, depth: f64, scale: f64, k: usize, seed: u64, min_y: i32 },
}

impl Shape {
    fn floor(&self, x: i32, z: i32, min_y: i32) -> Option<i32> {
        match self {
            Shape::Cylinder { cx, cz, r } => {
                let dx = f64::from(x) + 0.5 - cx;
                let dz = f64::from(z) + 0.5 - cz;
                (dx * dx + dz * dz <= r * r).then_some(min_y - 1)
            }
            Shape::Gungnir(g) => g.floor(x, z),
            Shape::Crater { i, nx, nz, r, depth, ground, min_y, .. } => {
                let d = (f64::from(x) + 0.5 - nx).hypot(f64::from(z) + 0.5 - nz);
                let edge = r * (1.0 + 0.07 * ((f64::from(z) - nz).atan2(f64::from(x) - nx) * 5.0 + *i as f64).sin());
                if d > edge {
                    return None;
                }
                let k = d / edge;
                let y = f64::from(*ground) - depth * (1.0 - k * k) * (1.0 - 0.3 * k * k);
                Some((y.floor() as i32).max(*min_y))
            }
            Shape::Link { a, b, ga, gb, w, depth, min_y, .. } => {
                let (off, u) = against(*a, *b, f64::from(x) + 0.5, f64::from(z) + 0.5);
                if off > *w {
                    return None;
                }
                let g = f64::from(*ga) + (f64::from(*gb) - f64::from(*ga)) * u;
                let s = off / w;
                Some(((g - depth * (1.0 - s * s * s * s)).floor() as i32).max(*min_y))
            }
        }
    }

    fn surface(&self, x: i32, z: i32) -> Option<&'static str> {
        match self {
            Shape::Cylinder { .. } => None,
            Shape::Gungnir(g) => g.scorched(x, z),
            Shape::Crater { nx, nz, r, seed, .. } => {
                let k = (f64::from(x) + 0.5 - nx).hypot(f64::from(z) + 0.5 - nz) / r
                    + (sk::hash(*seed, i64::from(x), i64::from(z)) - 0.5) * 0.18;
                let roll = sk::hash(*seed, i64::from(z) * 3 + 7, i64::from(x) * 5 - 1);
                if k < 0.25 {
                    Some(if roll < 0.5 {
                        "minecraft:crying_obsidian"
                    } else if roll < 0.8 {
                        "minecraft:obsidian"
                    } else {
                        "minecraft:amethyst_block"
                    })
                } else if k < 0.7 {
                    Some(if roll < 0.35 {
                        "minecraft:amethyst_block"
                    } else if roll < 0.55 {
                        "minecraft:calcite"
                    } else if roll < 0.8 {
                        "minecraft:blackstone"
                    } else {
                        "minecraft:smooth_basalt"
                    })
                } else if roll < 0.3 {
                    Some("minecraft:calcite")
                } else if roll < 0.55 {
                    Some("minecraft:tuff")
                } else if roll < 0.8 {
                    Some("minecraft:cobbled_deepslate")
                } else {
                    None
                }
            }
            Shape::Link { a, b, scale, k, seed, .. } => {
                let (off, _) = against(*a, *b, f64::from(x) + 0.5, f64::from(z) + 0.5);
                if off < scale.max(0.6) {
                    return Some(STAR_TRACE);
                }
                let roll = sk::hash(*seed, i64::from(x) * 11 + *k as i64, i64::from(z) * 7 - *k as i64);
                Some(if roll < 0.3 {
                    "minecraft:blackstone"
                } else if roll < 0.5 {
                    "minecraft:amethyst_block"
                } else if roll < 0.75 {
                    "minecraft:smooth_basalt"
                } else {
                    "minecraft:calcite"
                })
            }
        }
    }
}

fn against(a: [f64; 2], b: [f64; 2], x: f64, z: f64) -> (f64, f64) {
    let ex = b[0] - a[0];
    let ez = b[1] - a[1];
    let u = (((x - a[0]) * ex + (z - a[1]) * ez) / (ex * ex + ez * ez).max(1e-9)).clamp(0.0, 1.0);
    ((x - a[0] - ex * u).hypot(z - a[1] - ez * u), u)
}

/// A cut through the land, chunk by chunk, nearest first: the original's
/// `Carving`, with Horizon's bulk edit for its sections and chunk tickets
/// for the land not loaded yet.
struct Carving {
    owner: String,
    shape: Shape,
    chunks: Vec<[i32; 2]>,
    reach: Vec<f64>,
    next: usize,
    waiting: Vec<usize>,
    held: HashSet<usize>,
    per_tick: usize,
    spared: Option<[i32; 3]>,
    min_y: i32,
    max_y: i32,
    removed: u64,
    sweeps: u32,
}

impl Carving {
    /// The chunks within `radius` of `centre`, nearest first: by their
    /// centres for the beam, by their nearest corner for a crater.
    #[allow(clippy::too_many_arguments)]
    fn new(
        owner: String,
        shape: Shape,
        centre: [f64; 3],
        radius: f64,
        by_centre: bool,
        per_tick: usize,
        spared: Option<[i32; 3]>,
        min_y: i32,
        max_y: i32,
    ) -> Carving {
        let (cx, cz) = (centre[0], centre[2]);
        let c0x = ((cx - radius).floor() as i32) >> 4;
        let c1x = ((cx + radius).floor() as i32) >> 4;
        let c0z = ((cz - radius).floor() as i32) >> 4;
        let c1z = ((cz + radius).floor() as i32) >> 4;
        let mut list = Vec::new();
        for x in c0x..=c1x {
            for z in c0z..=c1z {
                let bx = f64::from(x) * 16.0;
                let bz = f64::from(z) * 16.0;
                let nx = cx.clamp(bx, bx + 16.0) - cx;
                let nz = cz.clamp(bz, bz + 16.0) - cz;
                let near = (nx * nx + nz * nz).sqrt();
                if near <= radius {
                    let order = if by_centre { (bx + 8.0 - cx).hypot(bz + 8.0 - cz) } else { near };
                    list.push(([x, z], order));
                }
            }
        }
        list.sort_by(|a, b| a.1.total_cmp(&b.1));
        Carving {
            owner,
            shape,
            chunks: list.iter().map(|(c, _)| *c).collect(),
            reach: list.iter().map(|(_, r)| *r).collect(),
            next: 0,
            waiting: Vec::new(),
            held: HashSet::new(),
            per_tick,
            spared,
            min_y,
            max_y,
            removed: 0,
            sweeps: 0,
        }
    }

    fn done(&self) -> bool {
        self.next >= self.chunks.len() && self.waiting.is_empty()
    }

    /// Cut up to the tick's share of chunks out to `front`; the chunks cut.
    fn tick(&mut self, game: &mut super::Game<'_>, dimension: &str, front: f64) -> Vec<[i32; 2]> {
        let mut cut = Vec::new();
        let mut budget = self.per_tick;
        self.sweeps += 1;
        let all = self.sweeps % 20 == 0;
        let mut index = 0;
        while index < self.waiting.len() && budget > 0 {
            let i = self.waiting[index];
            if all || self.held.contains(&i) {
                let chunk = self.chunks[i];
                if self.cut(game, dimension, chunk) {
                    self.waiting.remove(index);
                    if self.held.remove(&i) {
                        release(game, &self.owner, chunk);
                    }
                    cut.push(chunk);
                    budget -= 1;
                    continue;
                }
            }
            index += 1;
        }
        while budget > 0 && self.next < self.chunks.len() && self.reach[self.next] <= front {
            let chunk = self.chunks[self.next];
            if self.cut(game, dimension, chunk) {
                cut.push(chunk);
                budget -= 1;
            } else {
                self.waiting.push(self.next);
            }
            self.next += 1;
        }
        // Ask for the land still missing, a window at a time.
        for k in 0..self.waiting.len() {
            if self.held.len() >= 64 {
                break;
            }
            let i = self.waiting[k];
            if !self.held.contains(&i) && hold(game, &self.owner, self.chunks[i]) {
                self.held.insert(i);
            }
        }
        if self.done() && !self.chunks.is_empty() && !cut.is_empty() {
            log::info!("[{ID}] a cut of {} chunks is done: {} blocks taken", self.chunks.len(), self.removed);
        }
        cut
    }

    fn release_all(&mut self, game: &mut super::Game<'_>) {
        for i in std::mem::take(&mut self.held) {
            release(game, &self.owner, self.chunks[i]);
        }
    }

    /// Cut one chunk: false when it isn't loaded.
    fn cut(&mut self, game: &mut super::Game<'_>, dimension: &str, chunk: [i32; 2]) -> bool {
        let mut columns = Vec::with_capacity(256);
        for lx in 0..16 {
            for lz in 0..16 {
                columns.push((chunk[0] * 16 + lx, chunk[1] * 16 + lz));
            }
        }
        let Ok(heights) = game.native_surface_heights(dimension, &columns, NativeHeightmap::WorldSurface) else {
            return false;
        };
        if heights.iter().all(Option::is_none) {
            return false;
        }
        let mut edit = Edit::default();
        for ((x, z), height) in columns.iter().zip(heights.iter()) {
            if let Some(s) = self.spared
                && (x - s[0]).abs() <= 2
                && (z - s[2]).abs() <= 2
            {
                continue;
            }
            let Some(floor) = self.shape.floor(*x, *z, self.min_y) else {
                continue;
            };
            if floor >= self.max_y {
                continue;
            }
            let top = height.unwrap_or(self.max_y + 1);
            if top > floor + 1 {
                let from = floor + 1;
                edit.column(*x, *z, from, (self.max_y - floor) as u32, AIR);
                self.removed += (top - from).max(0) as u64;
            }
            if let Some(block) = self.shape.surface(*x, *z)
                && top > floor
                && floor >= self.min_y
            {
                edit.column(*x, *z, floor, 1, block);
            }
        }
        apply(game, dimension, &edit);
        true
    }
}

/// Hold a chunk loaded for `owner`, within everyone's share.
fn hold(game: &mut super::Game<'_>, owner: &str, chunk: [i32; 2]) -> bool {
    if HOLDS.with(Cell::get) >= MAX_HOLDS {
        return false;
    }
    let held = game.native_force_chunk(owner, chunk, true);
    if held {
        HOLDS.with(|h| h.set(h.get() + 1));
    }
    held
}

fn release(game: &mut super::Game<'_>, owner: &str, chunk: [i32; 2]) {
    game.native_force_chunk(owner, chunk, false);
    HOLDS.with(|h| h.set(h.get().saturating_sub(1)));
}

// ---- bulk edits ---------------------------------------------------------------------------------------------------

/// A bulk edit being built: blocks by name, and the runs that place them.
#[derive(Default)]
pub struct Edit {
    palette: Vec<String>,
    runs: Vec<NativeBlockRun>,
}

impl Edit {
    fn index(&mut self, block: &str) -> u32 {
        if let Some(index) = self.palette.iter().position(|known| known == block) {
            return index as u32;
        }
        self.palette.push(block.to_owned());
        (self.palette.len() - 1) as u32
    }

    /// `height` blocks of `block` up from `y` in one column.
    pub fn column(&mut self, x: i32, z: i32, y: i32, height: u32, block: &str) {
        if height == 0 {
            return;
        }
        let block = self.index(block);
        self.runs.push(NativeBlockRun {
            x,
            z,
            y,
            height,
            surface: None,
            block,
            only: None,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

/// Change blocks for the spell whose tick is running: nothing in its
/// caster's safe mode, and what is overwritten goes in the undo journal first.
/// Edits made outside a spell's tick (the city's) are neither.
pub fn apply(game: &mut super::Game<'_>, dimension: &str, edit: &Edit) -> Option<u64> {
    if edit.is_empty() {
        return None;
    }
    if let Some(context) = CONTEXT.with(|c| c.borrow().clone()) {
        if !context.terrain {
            return None;
        }
        if context.skill != Skill::Gravitia {
            record(game, &context, dimension, edit);
        }
    }
    apply_raw(game, dimension, edit)
}

/// The bulk edit itself, not journaled.
fn apply_raw(game: &mut super::Game<'_>, dimension: &str, edit: &Edit) -> Option<u64> {
    let bulk = NativeBlockEdit {
        palette: edit.palette.clone(),
        filters: Vec::new(),
        runs: edit.runs.clone(),
    };
    match game.native_edit_blocks(dimension, &bulk) {
        Ok(changed) => Some(changed),
        Err(error) => {
            log::warn!("[{ID}] an edit was left out: {error}");
            None
        }
    }
}

// ---- the undo journal ---------------------------------------------------------------------------------------------

/// Who is editing: the spell whose tick is running.
#[derive(Clone)]
struct Context {
    spell: u64,
    caster: u128,
    skill: Skill,
    terrain: bool,
}

fn enter(spell: &Spell) {
    CONTEXT.with(|c| {
        *c.borrow_mut() = Some(Context { spell: spell.id, caster: spell.caster, skill: spell.skill, terrain: spell.terrain });
    });
}

fn leave() {
    CONTEXT.with(|c| *c.borrow_mut() = None);
}

/// The most spells of a player kept for `/tss undo`.
const JOURNAL_KEEP: usize = 3;
/// The chunks' worth of columns an undo puts back each tick.
const RESTORE_CHUNKS: usize = 16;
/// The ticks an undo waits for a chunk that is not loaded.
const RESTORE_PATIENCE: u16 = 600;

/// One stretch of a column: `len` blocks up from `y`, run-length encoded as
/// (palette index, count).
struct Segment {
    y: i32,
    len: u32,
    runs: Vec<(u16, u16)>,
}

/// The blocks a spell overwrote, column by column, in a palette of their own.
/// Block states are not kept (the game's reads name only the block), so
/// stairs, logs and the like come back in their default orientation, and the
/// contents of chests and other block entities are lost.
#[derive(Default)]
struct Land {
    palette: Vec<String>,
    columns: HashMap<(i32, i32), Vec<Segment>>,
}

impl Land {
    fn intern(&mut self, block: &str) -> u16 {
        if let Some(at) = self.palette.iter().position(|known| known == block) {
            return at as u16;
        }
        self.palette.push(block.to_owned());
        (self.palette.len() - 1) as u16
    }

    /// The parts of `a..b` in a column that are not recorded yet.
    fn missing(&self, x: i32, z: i32, a: i32, b: i32) -> Vec<(i32, i32)> {
        let mut gaps = vec![(a, b)];
        for seg in self.columns.get(&(x, z)).map(Vec::as_slice).unwrap_or_default() {
            let (s0, s1) = (seg.y, seg.y + seg.len as i32);
            let mut next = Vec::with_capacity(gaps.len() + 1);
            for (g0, g1) in gaps {
                if s1 <= g0 || s0 >= g1 {
                    next.push((g0, g1));
                    continue;
                }
                if g0 < s0 {
                    next.push((g0, s0));
                }
                if s1 < g1 {
                    next.push((s1, g1));
                }
            }
            gaps = next;
        }
        gaps
    }

    /// Keep `ids` (palette indices) for the blocks up from `y`. The caller
    /// asks `missing` first: what is recorded is never written over.
    fn store(&mut self, x: i32, z: i32, y: i32, ids: &[u16]) {
        if ids.is_empty() {
            return;
        }
        let mut runs: Vec<(u16, u16)> = Vec::new();
        for &id in ids {
            match runs.last_mut() {
                Some((known, count)) if *known == id && *count < u16::MAX => *count += 1,
                _ => runs.push((id, 1)),
            }
        }
        let segs = self.columns.entry((x, z)).or_default();
        if let Some(last) = segs.last_mut()
            && last.y + last.len as i32 == y
        {
            last.len += ids.len() as u32;
            last.runs.extend(runs);
            return;
        }
        segs.push(Segment { y, len: ids.len() as u32, runs });
    }

    /// Record what `read` says is in `a..b` of a column, where the column's
    /// first `row` entry is the block at `row_y`; air above the row.
    fn record(&mut self, x: i32, z: i32, (a, b): (i32, i32), row_y: i32, row: &[u16], names: &[String]) {
        let air = self.intern(AIR);
        let mut remap: Vec<Option<u16>> = vec![None; names.len()];
        for (ya, yb) in self.missing(x, z, a, b) {
            let mut ids = Vec::with_capacity((yb - ya) as usize);
            for y in ya..yb {
                let id = match usize::try_from(y - row_y).ok().and_then(|k| row.get(k)) {
                    Some(&read) => match remap.get_mut(usize::from(read)) {
                        Some(slot) => *slot.get_or_insert_with(|| self.intern_name(names, read)),
                        None => air,
                    },
                    None => air,
                };
                ids.push(id);
            }
            self.store(x, z, ya, &ids);
        }
    }

    fn intern_name(&mut self, names: &[String], read: u16) -> u16 {
        let name = names.get(usize::from(read)).map(String::as_str).unwrap_or(AIR);
        self.intern(name)
    }

    fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// The runs of a column to write back: (y, blocks high, palette index).
    fn runs(&self, x: i32, z: i32) -> Vec<(i32, u32, u16)> {
        let mut out = Vec::new();
        for seg in self.columns.get(&(x, z)).map(Vec::as_slice).unwrap_or_default() {
            let mut y = seg.y;
            for &(id, count) in &seg.runs {
                out.push((y, u32::from(count), id));
                y += i32::from(count);
            }
        }
        out
    }
}

/// A spell's journal.
struct Journal {
    spell: u64,
    skill: Skill,
    dimension: String,
    done: bool,
    land: Land,
}

fn open_journal(spell: &Spell) {
    JOURNALS.with(|j| {
        let mut all = j.borrow_mut();
        let list = all.entry(spell.caster).or_default();
        list.push(Journal { spell: spell.id, skill: spell.skill, dimension: spell.dimension.clone(), done: false, land: Land::default() });
        // The oldest finished one goes; a spell still running never does.
        while list.len() > JOURNAL_KEEP {
            let Some(old) = list.iter().position(|journal| journal.done) else { break };
            list.remove(old);
        }
    });
}

fn close_journal(spell: &Spell) {
    JOURNALS.with(|j| {
        if let Some(journal) = j.borrow_mut().get_mut(&spell.caster).and_then(|l| l.iter_mut().find(|j| j.spell == spell.id)) {
            journal.done = true;
        }
    });
}

/// Keep the blocks `edit` is about to overwrite, those not kept before.
fn record(game: &super::Game<'_>, context: &Context, dimension: &str, edit: &Edit) {
    JOURNALS.with(|journals| {
        let mut journals = journals.borrow_mut();
        let Some(journal) = journals.get_mut(&context.caster).and_then(|l| l.iter_mut().find(|j| j.spell == context.spell)) else {
            return;
        };
        let (min_y, height) = game.native_world_height();
        let top = min_y + height;
        let mut wanted: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
        for run in &edit.runs {
            let (a, b) = (run.y.max(min_y), run.y.saturating_add(run.height as i32).min(top));
            if a < b && !journal.land.missing(run.x, run.z, a, b).is_empty() {
                wanted.entry((run.x, run.z)).or_default().push((a, b));
            }
        }
        let mut columns: Vec<(i32, i32)> = wanted.keys().copied().collect();
        columns.sort_unstable();
        for batch in columns.chunks(256) {
            // Only as deep as the lowest block this edit writes (a burnt
            // surface reads a few blocks, a beam the whole column).
            let Ok(bases) = game.native_surface_heights(dimension, batch, NativeHeightmap::WorldSurface) else { continue };
            let deepest = batch
                .iter()
                .zip(&bases)
                .filter_map(|(c, base)| Some(base.as_ref()? - wanted[c].iter().map(|r| r.0).min()?))
                .max()
                .unwrap_or(0);
            if deepest <= 0 {
                // Everything written is over the ground: it was air.
                for column in batch {
                    for &range in &wanted[column] {
                        journal.land.record(column.0, column.1, range, i32::MAX, &[], &[]);
                    }
                }
                continue;
            }
            let below = deepest.min(height).max(1);
            let rows = below as usize;
            let read = match game.native_read_columns(dimension, batch, NativeHeightmap::WorldSurface, (below as u32, 0)) {
                Ok(read) if read.bases.len() == batch.len() && read.blocks.len() == batch.len() * rows => read,
                Ok(_) => continue,
                Err(error) => {
                    log::warn!("[{ID}] land for the undo could not be read: {error}");
                    continue;
                }
            };
            for (i, column) in batch.iter().enumerate() {
                // Not loaded: the edit will not land either.
                let Some(base) = read.bases[i] else { continue };
                let row = &read.blocks[i * rows..(i + 1) * rows];
                for &range in &wanted[column] {
                    journal.land.record(column.0, column.1, range, base - below, row, &read.palette);
                }
            }
        }
    });
}

/// An undo being carried out: the chunks still to put back.
struct Restore {
    uuid: u128,
    dimension: String,
    title: &'static str,
    palette: Vec<String>,
    /// Chunks to restore, the next one last.
    chunks: Vec<ChunkWork>,
}

struct ChunkWork {
    chunk: [i32; 2],
    columns: Vec<((i32, i32), Vec<Segment>)>,
    tries: u16,
}

/// A land's columns grouped by chunk, the first to restore last.
fn by_chunk(land: Land) -> (Vec<String>, Vec<ChunkWork>) {
    let mut chunks: HashMap<[i32; 2], Vec<((i32, i32), Vec<Segment>)>> = HashMap::new();
    for (column, segs) in land.columns {
        chunks.entry([column.0 >> 4, column.1 >> 4]).or_default().push((column, segs));
    }
    let mut work: Vec<ChunkWork> = chunks.into_iter().map(|(chunk, columns)| ChunkWork { chunk, columns, tries: 0 }).collect();
    work.sort_by_key(|w| std::cmp::Reverse(w.chunk));
    (land.palette, work)
}

/// `/tss undo`: queue the restoring of the player's last finished spell.
pub fn undo(_game: &mut super::Game<'_>, uuid: u128) -> String {
    let taken = JOURNALS.with(|j| {
        let mut all = j.borrow_mut();
        let list = all.get_mut(&uuid)?;
        // A spell that changed nothing has nothing to undo.
        list.retain(|journal| !journal.done || !journal.land.is_empty());
        let newest = list.last()?;
        if !newest.done {
            return Some(Err(newest.skill));
        }
        list.pop().map(Ok)
    });
    match taken {
        None => "Nothing to undo".to_owned(),
        Some(Err(skill)) => format!("{} must finish first", skill.title()),
        Some(Ok(journal)) => {
            let title = journal.skill.title();
            let (palette, chunks) = by_chunk(journal.land);
            RESTORES.with(|r| r.borrow_mut().push(Restore { uuid, dimension: journal.dimension, title, palette, chunks }));
            format!("Restoring the land of {title}…")
        }
    }
}

/// Put back a few chunks of every undo in this level.
fn restore_tick(game: &mut super::Game<'_>, dimension: &str) {
    if RESTORES.with(|r| r.borrow().iter().all(|restore| restore.dimension != dimension)) {
        return;
    }
    let mut restores = RESTORES.with(|r| std::mem::take(&mut *r.borrow_mut()));
    restores.retain_mut(|restore| {
        if restore.dimension != dimension {
            return true;
        }
        let mut budget = RESTORE_CHUNKS;
        // Chunks not loaded wait at the back of the queue, to be asked again.
        let mut looked = RESTORE_CHUNKS * 4;
        let mut edit = Edit::default();
        while budget > 0 && looked > 0 {
            let Some(mut work) = restore.chunks.pop() else { break };
            looked -= 1;
            let first = work.columns.first().map(|(column, _)| *column);
            let loaded = first.is_some_and(|column| {
                game.native_surface_heights(dimension, &[column], NativeHeightmap::WorldSurface)
                    .is_ok_and(|heights| heights.first().is_some_and(Option::is_some))
            });
            if !loaded {
                work.tries += 1;
                if work.tries < RESTORE_PATIENCE {
                    restore.chunks.insert(0, work);
                } else {
                    log::warn!("[{ID}] the chunk {:?} never loaded: its land stays as it is", work.chunk);
                }
                continue;
            }
            budget -= 1;
            for ((x, z), segs) in &work.columns {
                for seg in segs {
                    let mut y = seg.y;
                    for &(id, count) in &seg.runs {
                        let block = restore.palette.get(usize::from(id)).map_or(AIR, String::as_str);
                        edit.column(*x, *z, y, u32::from(count), block);
                        y += i32::from(count);
                    }
                }
            }
        }
        if !edit.is_empty() {
            apply_raw(game, dimension, &edit);
        }
        if restore.chunks.is_empty() {
            game.native_action_bar(restore.uuid, crate::text::Text::literal("The land is restored".to_owned()));
            log::info!("[{ID}] the land of {} is restored", restore.title);
            return false;
        }
        true
    });
    RESTORES.with(|r| {
        let mut r = r.borrow_mut();
        restores.append(&mut r);
        *r = restores;
    });
}

/// Set single blocks (a spire, a star's crystal) in column runs.
fn paint(game: &mut super::Game<'_>, dimension: &str, blocks: &HashMap<[i32; 3], &'static str>) {
    let mut by_column: HashMap<(i32, i32), Vec<(i32, &'static str)>> = HashMap::new();
    for (at, block) in blocks {
        by_column.entry((at[0], at[2])).or_default().push((at[1], block));
    }
    let mut edit = Edit::default();
    for ((x, z), mut ys) in by_column {
        ys.sort_by_key(|(y, _)| *y);
        let mut run: Option<(i32, u32, &'static str)> = None;
        for (y, block) in ys {
            run = match run {
                Some((start, height, b)) if start + height as i32 == y && b == block => Some((start, height + 1, b)),
                Some((start, height, b)) => {
                    edit.column(x, z, start, height, b);
                    Some((y, 1, block))
                }
                None => Some((y, 1, block)),
            };
        }
        if let Some((start, height, b)) = run {
            edit.column(x, z, start, height, b);
        }
    }
    apply(game, dimension, &edit);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_remainder() {
        assert!((ieee_remainder(5.0, 3.0) - -1.0).abs() < 1e-12);
        assert!((ieee_remainder(4.0, 3.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_beam_takes_the_column_and_the_bedrock() {
        let shape = Shape::Cylinder { cx: 0.5, cz: 0.5, r: 10.0 };
        assert_eq!(shape.floor(0, 0, -64), Some(-65));
        assert_eq!(shape.floor(20, 0, -64), None);
    }

    #[test]
    fn gungnir_spares_its_spire_and_digs_its_bowl() {
        let g = GungnirShape::new([0.5, 70.0, 0.5], 42, 4.0, -64);
        assert_eq!(g.floor(0, 0), None);
        let mid = g.floor(30, 0).expect("in the crater");
        let rim = g.floor(380, 0).expect("in the land planed");
        assert!(mid < rim);
        assert!(g.floor(600, 0).is_none());
        assert!(!g.spire_blocks(319).is_empty());
    }

    #[test]
    fn a_carving_goes_nearest_first() {
        let c = Carving::new(String::new(), Shape::Cylinder { cx: 0.0, cz: 0.0, r: 40.0 }, [0.0; 3], 40.0, true, 12, None, -64, 319);
        assert!(c.reach.windows(2).all(|w| w[0] <= w[1]));
        assert!(c.chunks.contains(&[0, 0]) && c.chunks.contains(&[-1, -1]));
    }

    /// The ids of a column's recorded blocks from `a` to `b`, `None` where nothing is.
    fn recorded(land: &Land, x: i32, z: i32, a: i32, b: i32) -> Vec<Option<String>> {
        let mut out = vec![None; (b - a) as usize];
        for (y, height, id) in land.runs(x, z) {
            for k in y..y + height as i32 {
                if (a..b).contains(&k) {
                    out[(k - a) as usize] = Some(land.palette[usize::from(id)].clone());
                }
            }
        }
        out
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_journal_keeps_only_what_is_not_kept() {
        let mut land = Land::default();
        assert_eq!(land.missing(1, 2, 10, 20), vec![(10, 20)]);
        land.store(1, 2, 12, &[0, 0, 0]);
        assert_eq!(land.missing(1, 2, 10, 20), vec![(10, 12), (15, 20)]);
        assert_eq!(land.missing(1, 2, 12, 15), vec![]);
        assert_eq!(land.missing(3, 2, 10, 20), vec![(10, 20)]);
    }

    #[test]
    fn the_first_write_wins_and_the_runs_reproduce_the_land() {
        // The land, up from y = 0: stone x4, dirt x2, grass, then air above.
        let palette = names(&["minecraft:air", "minecraft:stone", "minecraft:dirt", "minecraft:grass_block"]);
        let row: Vec<u16> = vec![1, 1, 1, 1, 2, 2, 3];
        let mut land = Land::default();
        // A first edit takes y 3..6; the second, over y 2..9, reads the land
        // as the first left it (air from 3 up) but may only add y 2 and 6..9.
        land.record(5, 5, (3, 6), 0, &row, &palette);
        let after: Vec<u16> = vec![1, 1, 1, 0, 0, 0, 0];
        land.record(5, 5, (2, 9), 0, &after, &palette);
        let got = recorded(&land, 5, 5, 2, 9);
        let got: Vec<&str> = got.iter().map(|b| b.as_deref().unwrap_or("none")).collect();
        let want = ["stone", "stone", "dirt", "dirt", "air", "air", "air"].map(|n| format!("minecraft:{n}"));
        // y 2 comes from the second read; y 3..6 are the first write's.
        assert_eq!(got, want.iter().map(String::as_str).collect::<Vec<_>>());
        // The runs are compact.
        assert!(land.columns[&(5, 5)].iter().map(|s| s.runs.len()).sum::<usize>() <= 8);
    }

    #[test]
    fn long_runs_split_and_come_back_whole() {
        let mut land = Land::default();
        let ids = vec![7u16; 70_000];
        land.store(0, 0, -64, &ids);
        let total: u32 = land.runs(0, 0).iter().map(|r| r.1).sum();
        assert_eq!(total, 70_000);
        assert_eq!(land.runs(0, 0)[1].0, -64 + 65_535);
    }

    #[test]
    fn a_journal_groups_its_columns_by_chunk() {
        let mut land = Land::default();
        land.store(-1, 0, 0, &[1]);
        land.store(0, 0, 0, &[1]);
        land.store(15, 15, 0, &[1]);
        let (_, work) = by_chunk(land);
        assert_eq!(work.len(), 2);
        assert_eq!(work.iter().map(|w| w.columns.len()).sum::<usize>(), 3);
    }
}
