// SS-06 Gravitia's Imaginary Space: the "endless geometric city in the void"
// the original draws in its own dimension (`gravitia_space_city.glsl`), built
// here out of real blocks. Horizon runs only the three vanilla dimensions, so
// the world is not moved into another one: the level Gravitia was cast in
// becomes Imaginary Space where it stands, every chunk of it, now and every
// time one loads, and it stays so (the world's folder remembers it).
//
// The layout is the shader's, function for function, on the same hashes: big
// boxes in 40x36x40 cells above a ceiling and below a floor round the level's
// middle, smaller ones in 14x12x14 cells where they don't crowd them.
use super::shooting_star::ID;
use super::shooting_star_spells as spells;
use crate::game::native_world_edit::NativeHeightmap;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write as _;

const BX: i32 = 40;
const BY: i32 = 36;
const BZ: i32 = 40;
const BIG_MARGIN: i32 = 3;
const SX: i32 = 14;
const SY: i32 = 12;
const SZ: i32 = 14;
const SMALL_MARGIN: i32 = 2;
const BIG_FILL: u32 = 55;
const SMALL_FILL: u32 = 22;
const FLUSH: u32 = 60;
const MID: i32 = 585;
const HALF: i32 = 53;
const NO_ROW: i32 = -1_000_000;

/// The chunks made Imaginary Space a tick, and how far round each player.
const PER_TICK: usize = 6;
const AROUND: i32 = 10;

/// The city's seed and the level's floor and ceiling (`City` in the shader).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct City {
    pub seed: u32,
    pub min_y: i32,
    pub max_y: i32,
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

fn next(h: u32) -> u32 {
    hash(h.wrapping_add(0x9e37_79b9))
}

fn pick(h: u32, n: i32) -> i32 {
    (h % n.max(1) as u32) as i32
}

fn div(a: i32, b: i32) -> i32 {
    a.div_euclid(b)
}

/// One box: its corners (`hi` exclusive) and how it looks (0 plain, 1 lit
/// lattice, 2 a star's core).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Box3 {
    pub lo: [i32; 3],
    pub hi: [i32; 3],
    pub look: i32,
}

impl City {
    fn cell(&self, c: [i32; 3], salt: u32) -> u32 {
        hash(
            (c[0] as u32)
                .wrapping_mul(0x8da6_b343)
                .wrapping_add((c[1] as u32).wrapping_mul(0xd816_3841))
                .wrapping_add((c[2] as u32).wrapping_mul(0xcb1a_b31f))
                .wrapping_add(self.seed)
                .wrapping_add(salt.wrapping_mul(0x2545_f491)),
        )
    }

    fn span(&self) -> i32 {
        (self.max_y - self.min_y).max(0)
    }

    fn mid(&self) -> i32 {
        self.min_y + self.span() * MID / 1000
    }

    fn half(&self) -> i32 {
        (self.span() * HALF / 1000).max(4)
    }

    fn jitter(&self) -> i32 {
        (self.half() * 3 / 10).max(1)
    }

    fn floor(&self, c: [i32; 2]) -> i32 {
        let j = self.jitter();
        self.mid() - self.half() + pick(self.cell([c[0], 0, c[1]], 3), 2 * j + 1) - j
    }

    fn ceiling(&self, c: [i32; 2]) -> i32 {
        let j = self.jitter();
        self.mid() + self.half() + pick(next(self.cell([c[0], 0, c[1]], 3)), 2 * j + 1) - j
    }

    fn row(&self, c: [i32; 2], y: i32) -> i32 {
        let f = self.floor(c);
        if y < f {
            return -1 - (f - 1 - y) / BY;
        }
        let ce = self.ceiling(c);
        if y >= ce { (y - ce) / BY } else { NO_ROW }
    }

    fn clip(&self, b: &mut Box3) -> bool {
        b.lo[1] = b.lo[1].max(self.min_y);
        b.hi[1] = b.hi[1].min(self.max_y + 1);
        b.lo[1] < b.hi[1]
    }

    fn look(h: u32) -> i32 {
        let k = pick(h, 100);
        if k < 85 {
            0
        } else if k < 97 {
            1
        } else {
            2
        }
    }

    /// The big box of cell `c` (x, row, z), if it has one.
    pub fn big(&self, c: [i32; 3]) -> Option<Box3> {
        let flush_row = c[1] == -1 || c[1] == 0;
        let h = self.cell(c, 1);
        if !flush_row && pick(h, 100) as u32 >= BIG_FILL {
            return None;
        }
        let r1 = next(h);
        let r2 = next(r1);
        let r3 = next(r2);
        let r4 = next(r3);
        let r5 = next(r4);
        let r6 = next(r5);
        let r7 = next(r6);
        let r8 = next(r7);
        let r9 = next(r8);
        let kind = pick(r1, 100);
        let (w, d, t) = if kind < 10 {
            (8 + pick(r2, 7), 8 + pick(r3, 7), 22 + pick(r4, 9))
        } else if kind < 25 {
            (16 + pick(r2, 19), 16 + pick(r3, 19), 2 + pick(r4, 3))
        } else {
            (10 + pick(r2, 23), 10 + pick(r3, 23), 6 + pick(r4, 25))
        };
        let lo_x = c[0] * BX + BIG_MARGIN + pick(r5, BX - 2 * BIG_MARGIN - w + 1);
        let lo_z = c[2] * BZ + BIG_MARGIN + pick(r6, BZ - 2 * BIG_MARGIN - d + 1);
        let bottom = if c[1] < 0 {
            self.floor([c[0], c[2]]) + c[1] * BY
        } else {
            self.ceiling([c[0], c[2]]) + c[1] * BY
        };
        let mut lo_y = bottom + BIG_MARGIN + pick(r7, BY - 2 * BIG_MARGIN - t + 1);
        if flush_row && (pick(r9, 100) as u32) < FLUSH {
            lo_y = if c[1] < 0 { bottom + BY - t } else { bottom };
        }
        let mut b = Box3 {
            lo: [lo_x, lo_y, lo_z],
            hi: [lo_x + w, lo_y + t, lo_z + d],
            look: Self::look(r8),
        };
        self.clip(&mut b).then_some(b)
    }

    fn crowded(&self, lo: [i32; 3], hi: [i32; 3]) -> bool {
        let a = [div(lo[0] - 1, BX), div(lo[2] - 1, BZ)];
        let b = [div(hi[0], BX), div(hi[2], BZ)];
        for i in 0..2 {
            for k in 0..2 {
                let c = [a[0] + i, a[1] + k];
                if c[0] > b[0] || c[1] > b[1] {
                    continue;
                }
                if hi[1] + 1 > self.floor(c) && lo[1] - 1 < self.ceiling(c) {
                    return true;
                }
                let r0 = self.row(c, lo[1] - 1);
                let r1 = self.row(c, hi[1]);
                for j in 0..2 {
                    let cy = r0 + j;
                    if cy > r1 {
                        continue;
                    }
                    if let Some(big) = self.big([c[0], cy, c[1]])
                        && (0..3).all(|n| lo[n] - 1 < big.hi[n])
                        && (0..3).all(|n| hi[n] + 1 > big.lo[n])
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// The small box of small cell `c`, if it has one that isn't crowded.
    pub fn small(&self, c: [i32; 3]) -> Option<Box3> {
        let h = self.cell(c, 2);
        if pick(h, 100) as u32 >= SMALL_FILL {
            return None;
        }
        let r1 = next(h);
        let r2 = next(r1);
        let r3 = next(r2);
        let r4 = next(r3);
        let r5 = next(r4);
        let r6 = next(r5);
        let r7 = next(r6);
        let w = 2 + pick(r1, 6);
        let t = 2 + pick(r2, 6);
        let d = 2 + pick(r3, 6);
        let lo = [
            c[0] * SX + SMALL_MARGIN + pick(r4, SX - 2 * SMALL_MARGIN - w + 1),
            c[1] * SY + SMALL_MARGIN + pick(r5, SY - 2 * SMALL_MARGIN - t + 1),
            c[2] * SZ + SMALL_MARGIN + pick(r6, SZ - 2 * SMALL_MARGIN - d + 1),
        ];
        let hi = [lo[0] + w, lo[1] + t, lo[2] + d];
        if self.crowded(lo, hi) {
            return None;
        }
        let mut b = Box3 { lo, hi, look: Self::look(r7) };
        self.clip(&mut b).then_some(b)
    }

    /// Every box with a block in the chunk.
    pub fn boxes_in(&self, chunk: [i32; 2]) -> Vec<Box3> {
        let x0 = chunk[0] * 16;
        let z0 = chunk[1] * 16;
        let (x1, z1) = (x0 + 15, z0 + 15);
        let mut out = Vec::new();
        let touches = |b: &Box3| b.hi[0] > x0 && b.lo[0] <= x1 && b.hi[2] > z0 && b.lo[2] <= z1;
        for cx in div(x0, BX)..=div(x1, BX) {
            for cz in div(z0, BZ)..=div(z1, BZ) {
                let low = self.row([cx, cz], self.min_y);
                let high = self.row([cx, cz], self.max_y);
                let rows = (low.min(-1)..=-1).chain(0..=high.max(-1));
                for cy in rows {
                    if let Some(b) = self.big([cx, cy, cz])
                        && touches(&b)
                    {
                        out.push(b);
                    }
                }
            }
        }
        for cx in div(x0, SX)..=div(x1, SX) {
            for cz in div(z0, SZ)..=div(z1, SZ) {
                for cy in div(self.min_y, SY)..=div(self.max_y, SY) {
                    if let Some(b) = self.small([cx, cy, cz])
                        && touches(&b)
                    {
                        out.push(b);
                    }
                }
            }
        }
        out
    }

    /// Where a player stands safe: on top of the box under the street of
    /// their cell.
    pub fn street(&self, x: f64, z: f64) -> [f64; 3] {
        let c = [div(x.floor() as i32, BX), div(z.floor() as i32, BZ)];
        match self.big([c[0], -1, c[1]]) {
            Some(b) => {
                let tx = (x.floor() as i32).clamp(b.lo[0] + 1, (b.hi[0] - 2).max(b.lo[0] + 1));
                let tz = (z.floor() as i32).clamp(b.lo[2] + 1, (b.hi[2] - 2).max(b.lo[2] + 1));
                [f64::from(tx) + 0.5, f64::from(b.hi[1]), f64::from(tz) + 0.5]
            }
            None => [x, f64::from(self.mid()), z],
        }
    }
}

fn block_of(look: i32) -> &'static str {
    match look {
        1 => "shooting_star_demo:lattice_edge[hue=4]",
        2 => "shooting_star_demo:star_core",
        _ => "shooting_star_demo:gravitia_space_block",
    }
}

// ---- the world's memory -------------------------------------------------------------------------------------------

/// A level sent into Imaginary Space.
struct Banished {
    seed: u32,
    converted: HashSet<[i32; 2]>,
    queue: VecDeque<[i32; 2]>,
    queued: HashSet<[i32; 2]>,
    written: Vec<String>,
    ticks: u64,
}

thread_local! {
    static LEVELS: RefCell<HashMap<String, Banished>> = RefCell::new(HashMap::new());
    static FILE: RefCell<Option<std::path::PathBuf>> = const { RefCell::new(None) };
}

/// A world opened (or closed): read what it remembers of Imaginary Space.
pub fn open_world(world: Option<&std::path::Path>) {
    LEVELS.with(|l| l.borrow_mut().clear());
    let file = world.map(|w| w.join("shooting_star_demo").join("imaginary_space.txt"));
    if let Some(path) = &file
        && let Ok(text) = std::fs::read_to_string(path)
    {
        LEVELS.with(|levels| {
            let mut levels = levels.borrow_mut();
            for line in text.lines() {
                let words: Vec<&str> = line.split_whitespace().collect();
                match words.as_slice() {
                    ["banished", dim, seed] => {
                        if let Ok(seed) = seed.parse::<u32>() {
                            levels.entry((*dim).to_owned()).or_insert_with(|| Banished::new(seed));
                        }
                    }
                    ["c", dim, x, z] => {
                        if let (Some(level), Ok(x), Ok(z)) = (levels.get_mut(*dim), x.parse::<i32>(), z.parse::<i32>()) {
                            level.converted.insert([x, z]);
                        }
                    }
                    _ => {}
                }
            }
            if !levels.is_empty() {
                log::info!("[{ID}] Imaginary Space holds: {}", levels.keys().cloned().collect::<Vec<_>>().join(", "));
            }
        });
    }
    FILE.with(|f| *f.borrow_mut() = file);
}

impl Banished {
    fn new(seed: u32) -> Banished {
        Banished {
            seed,
            converted: HashSet::new(),
            queue: VecDeque::new(),
            queued: HashSet::new(),
            written: Vec::new(),
            ticks: 0,
        }
    }

    fn push(&mut self, chunk: [i32; 2]) {
        if !self.converted.contains(&chunk) && self.queued.insert(chunk) {
            self.queue.push_back(chunk);
        }
    }
}

fn append(lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let Some(path) = FILE.with(|f| f.borrow().clone()) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut file) => {
            for line in lines {
                let _ = writeln!(file, "{line}");
            }
        }
        Err(error) => log::warn!("[{ID}] Imaginary Space could not be remembered: {error}"),
    }
}

/// Every level sent into Imaginary Space, with its city's seed.
pub fn banished_levels() -> Vec<(String, u32)> {
    LEVELS.with(|l| l.borrow().iter().map(|(d, b)| (d.clone(), b.seed)).collect())
}

/// The message telling a window that `dimension` is Imaginary Space.
pub fn void_message(dimension: &str, seed: u32) -> Vec<u8> {
    let mut out = vec![super::shooting_star_skills::FX_VOID];
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(dimension.as_bytes());
    out
}

/// Whether `dimension` is Imaginary Space.
pub fn banished(dimension: &str) -> bool {
    LEVELS.with(|l| l.borrow().contains_key(dimension))
}

/// Gravitia's blow: the level becomes Imaginary Space, beginning round its
/// players, who are set down in the city.
pub fn banish(game: &mut super::Game<'_>, dimension: &str, seed: u32) {
    if banished(dimension) {
        return;
    }
    append(&[format!("banished {dimension} {seed}")]);
    LEVELS.with(|l| l.borrow_mut().insert(dimension.to_owned(), Banished::new(seed)));
    // Every window learns it, for the sky it draws there from now on.
    let message = void_message(dimension, seed);
    for player in game.native_server_players() {
        let _ = game.native_net_send_to_player(player.uuid, super::shooting_star::CHANNEL, &message);
    }
    queue_round_players(game, dimension);
    let (min_y, height) = game.native_world_height();
    let city = City { seed, min_y, max_y: min_y + height - 1 };
    for player in game.native_server_players() {
        if player.dimension != dimension {
            continue;
        }
        let at = city.street(player.position[0], player.position[2]);
        game.native_run_command(&format!(
            "execute in {dimension} run tp {} {:.2} {:.2} {:.2}",
            player.name, at[0], at[1], at[2]
        ));
        game.native_run_command(&format!("effect give {} minecraft:slow_falling 30 0 true", player.name));
        game.native_run_command(&format!("effect give {} minecraft:resistance 20 4 true", player.name));
    }
}

fn queue_round_players(game: &mut super::Game<'_>, dimension: &str) {
    let players: Vec<[i32; 2]> = game
        .native_server_players()
        .iter()
        .filter(|p| p.dimension == dimension)
        .map(|p| [(p.position[0].floor() as i32) >> 4, (p.position[2].floor() as i32) >> 4])
        .collect();
    LEVELS.with(|l| {
        let mut l = l.borrow_mut();
        let Some(level) = l.get_mut(dimension) else {
            return;
        };
        for centre in players {
            // Nearest first, ring by ring.
            for r in 0..=AROUND {
                for dx in -r..=r {
                    for dz in -r..=r {
                        if dx.abs().max(dz.abs()) == r {
                            level.push([centre[0] + dx, centre[1] + dz]);
                        }
                    }
                }
            }
        }
    });
}

/// A chunk loaded (`chunk_load`): in Imaginary Space it is made so too.
pub fn chunk_loaded(dimension: &str, chunk: [i32; 2]) {
    LEVELS.with(|l| {
        if let Some(level) = l.borrow_mut().get_mut(dimension) {
            level.push(chunk);
        }
    });
}

/// The level's tick: a few chunks more made Imaginary Space, and anyone
/// falling out of it set back on the city.
pub fn level_tick(game: &mut super::Game<'_>, dimension: &str) {
    let Some((seed, batch, ticks)) = LEVELS.with(|l| {
        let mut l = l.borrow_mut();
        let level = l.get_mut(dimension)?;
        level.ticks += 1;
        let mut batch = Vec::new();
        while batch.len() < PER_TICK {
            let Some(chunk) = level.queue.pop_front() else { break };
            level.queued.remove(&chunk);
            if !level.converted.contains(&chunk) {
                batch.push(chunk);
            }
        }
        Some((level.seed, batch, level.ticks))
    }) else {
        return;
    };
    let (min_y, height) = game.native_world_height();
    let city = City { seed, min_y, max_y: min_y + height - 1 };
    let mut done = Vec::new();
    for chunk in batch {
        if convert(game, dimension, &city, chunk) {
            done.push(chunk);
        }
    }
    let lines: Vec<String> = done.iter().map(|c| format!("c {dimension} {} {}", c[0], c[1])).collect();
    let flush = LEVELS.with(|l| {
        let mut l = l.borrow_mut();
        let level = l.get_mut(dimension)?;
        for chunk in &done {
            level.converted.insert(*chunk);
        }
        level.written.extend(lines);
        (ticks % 40 == 0).then(|| std::mem::take(&mut level.written))
    });
    if let Some(lines) = flush {
        append(&lines);
    }
    if ticks % 20 == 0 {
        queue_round_players(game, dimension);
        rescue(game, dimension, &city);
    }
}

/// Make one chunk Imaginary Space: the world taken out, the city put in.
fn convert(game: &mut super::Game<'_>, dimension: &str, city: &City, chunk: [i32; 2]) -> bool {
    let probe = [(chunk[0] * 16 + 8, chunk[1] * 16 + 8)];
    let loaded = game
        .native_surface_heights(dimension, &probe, NativeHeightmap::WorldSurface)
        .map(|h| h.first().copied().flatten().is_some())
        .unwrap_or(false);
    if !loaded {
        return false;
    }
    let height = (city.max_y - city.min_y + 1) as u32;
    let mut edit = spells::Edit::default();
    for lx in 0..16 {
        for lz in 0..16 {
            edit.column(chunk[0] * 16 + lx, chunk[1] * 16 + lz, city.min_y, height, "minecraft:air");
        }
    }
    let (x0, z0) = (chunk[0] * 16, chunk[1] * 16);
    for b in city.boxes_in(chunk) {
        let block = block_of(b.look);
        for x in b.lo[0].max(x0)..b.hi[0].min(x0 + 16) {
            for z in b.lo[2].max(z0)..b.hi[2].min(z0 + 16) {
                edit.column(x, z, b.lo[1], (b.hi[1] - b.lo[1]) as u32, block);
            }
        }
    }
    spells::apply(game, dimension, &edit).is_some()
}

/// Nobody falls out of Imaginary Space: whoever drops below the city is
/// set back on it.
fn rescue(game: &mut super::Game<'_>, dimension: &str, city: &City) {
    let mut lines = Vec::new();
    for player in game.native_server_players() {
        if player.dimension != dimension || player.game_mode == 3 || player.health <= 0.0 {
            continue;
        }
        if player.position[1] < f64::from(city.min_y + 4) {
            let at = city.street(player.position[0], player.position[2]);
            lines.push(format!("execute in {dimension} run tp {} {:.2} {:.2} {:.2}", player.name, at[0], at[1], at[2]));
            lines.push(format!("effect give {} minecraft:slow_falling 10 0 true", player.name));
        }
    }
    for line in lines {
        game.native_run_command(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CITY: City = City { seed: 0x1234_5678, min_y: -64, max_y: 319 };

    #[test]
    fn the_street_row_always_has_a_box() {
        for x in -5..5 {
            for z in -5..5 {
                assert!(CITY.big([x, -1, z]).is_some());
            }
        }
    }

    #[test]
    fn boxes_keep_inside_the_level_and_their_cells() {
        for b in CITY.boxes_in([3, -7]) {
            assert!(b.lo[1] >= CITY.min_y && b.hi[1] <= CITY.max_y + 1);
            assert!(b.lo.iter().zip(b.hi.iter()).all(|(lo, hi)| lo < hi));
        }
        assert!(!CITY.boxes_in([0, 0]).is_empty());
    }

    #[test]
    fn the_street_is_open_between_floor_and_ceiling() {
        for x in -3..3 {
            assert!(CITY.floor([x, 0]) < CITY.ceiling([x, 0]));
            assert_eq!(CITY.row([x, 0], CITY.mid()), NO_ROW);
        }
    }
}
