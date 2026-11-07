// The Shooting Star (Horizon port): the window's half, and what both halves
// share.
//
// This file is a support module the bundle adds to
// `crates/horizon-client/src/game/native_hooks.rs`, so the game sees it as
// `crate::game::native_hooks::shooting_star`. It holds the remote's keys and
// tuning (saved per skill, as the original's `shooting_star_demo_skills
// .properties`), its menu, tooltip and HUD, and each spell's film, sounds,
// light and camera jolt, played on the original's own timelines from the
// one message the server sends when a spell starts. The server's spells live
// in `shooting_star_spells`.
use super::shooting_star_skills::{self as sk, Skill, Tuning};

/// The mod's id: its key in `horizon-mods.txt` and its settings folder.
pub const ID: &str = "shooting_star";
pub const VERSION: &str = "1.5.2-hz13";

/// The Stellar Remote, declared by the companion content mod of the same
/// namespace (`shooting_star_demo`, data/shooting_star_demo/horizon/item).
pub const REMOTE_ID: &str = "shooting_star_demo:stellar_remote";

/// The command usages the mod adds to the server's tree.
pub const COMMANDS: &[&str] = &["tss info", "tss cast <skill:word>", "tss select <skill:word>", "tss next", "tss undo", "tss trailer", "tss trailer stop"];

/// The server to the window, and the window to the server.
pub const CHANNEL: &str = "shooting_star:fx";
pub const CH_CMD: &str = "shooting_star:cmd";

/// The mod's keys: name, label and default, as the mods page lists them. The
/// first seven are the original's (H menu, J cutscenes, K I U Y skills; O
/// for Gravitia), then the port's own SS-H1 on B; the tuning keys stand in
/// for its menu's sliders.
pub const KEYS: [(&str, &str, &str); 13] = [
    ("skill_menu", "Open skill menu", "key.keyboard.h"),
    ("cutscenes", "Skip / toggle spell cutscenes", "key.keyboard.j"),
    ("the_shooting_star", "The Shooting Star: SS-01 Railgun", "key.keyboard.k"),
    ("gungnir", "The Shooting Star: SS-03 Gungnir", "key.keyboard.i"),
    ("seven_stars", "The Shooting Star: SS-04 Seven Stars", "key.keyboard.u"),
    ("tesseract", "The Shooting Star: SS-05 Tesseract", "key.keyboard.y"),
    ("gravitia", "The Shooting Star: SS-06 Gravitia", "key.keyboard.o"),
    ("event_horizon", "The Shooting Star: SS-H1 Event Horizon (the port's own)", "key.keyboard.b"),
    ("supernova", "The Shooting Star: SS-H2 Supernova (the port's own)", "key.keyboard.v"),
    ("cycle", "Next skill", "key.keyboard.g"),
    ("power", "Selected skill: power (Shift: down)", "key.keyboard.n"),
    ("speed", "Selected skill: speed (Shift: down)", "key.keyboard.m"),
    ("damage", "Selected skill: damage (Shift: down)", "key.keyboard.z"),
];

// ---- the window's remote ------------------------------------------------------------------------------------------

thread_local! {
    /// The levels that are Imaginary Space, by dimension, with their city's seed (the server says).
    static VOID: std::cell::RefCell<std::collections::HashMap<String, u32>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

thread_local! {
    static SELECTED: std::cell::Cell<Skill> = const { std::cell::Cell::new(Skill::ShootingStar) };
    static TUNES: std::cell::RefCell<[Tuning; sk::SKILLS]> = std::cell::RefCell::new(Skill::ALL.map(Tuning::initial));
    static CUTSCENES: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Whether this player's spells may change blocks (safe mode when off).
    static TERRAIN: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// The effects' quality: 0 low, 1 medium, 2 high (the default), 3 ultra.
    static QUALITY: std::cell::Cell<u8> = const { std::cell::Cell::new(2) };
    static LOADED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The selection and tuning wait to reach the server.
    static DIRTY: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Each skill's recharge as the server last told it: ticks left, ticks in all.
    static COOLDOWNS: std::cell::RefCell<[(u32, u32); sk::SKILLS]> = const { std::cell::RefCell::new([(0, 1); sk::SKILLS]) };
    /// When the player last picked a skill, so the HUD can introduce it.
    static SHOWN: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    /// The menu row waiting for a key press, while it waits.
    static BINDING: std::cell::Cell<Option<u8>> = const { std::cell::Cell::new(None) };
    static SHIFT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A pick a screen's click made, waiting for a tick that can send it.
    static PENDING: std::cell::Cell<Option<u8>> = const { std::cell::Cell::new(None) };
    static TICKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SPELLS: std::cell::RefCell<Vec<ClientSpell>> = const { std::cell::RefCell::new(Vec::new()) };
    static SHAKES: std::cell::RefCell<Vec<Jolt>> = const { std::cell::RefCell::new(Vec::new()) };
    static DREW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn selected() -> Skill {
    SELECTED.with(std::cell::Cell::get)
}

pub fn tune(skill: Skill) -> Tuning {
    TUNES.with(|t| t.borrow()[skill.index() as usize])
}

fn set_tune(skill: Skill, tune: Tuning) {
    TUNES.with(|t| t.borrow_mut()[skill.index() as usize] = tune.clamped(skill));
    DIRTY.with(|d| d.set(true));
}

/// Pick a skill, as `ClientSkills.select`: its chime and its name.
fn select(game: &mut super::Game<'_>, skill: Skill) {
    SELECTED.with(|s| s.set(skill));
    SHOWN.with(|s| s.set(Some(std::time::Instant::now())));
    DIRTY.with(|d| d.set(true));
    if let Some(eye) = camera(game) {
        game.native_play_local_sound("shooting_star_demo:staff_select", eye, 0.8, 0.9 + f32::from(skill.index()) * 0.05, 0);
    }
    game.native_client_action_bar(crate::text::Text::literal(format!("✦ {}", skill.title())), true);
    save(game);
}

fn settings_path(game: &super::Game<'_>) -> std::path::PathBuf {
    game.native_data_dir(ID).join("skills.properties")
}

fn load(game: &super::Game<'_>) {
    if LOADED.with(|l| l.replace(true)) {
        return;
    }
    let Ok(text) = std::fs::read_to_string(settings_path(game)) else {
        return;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        if key == "cutscenes" {
            CUTSCENES.with(|c| c.set(value != "false"));
        } else if key == "terrain" {
            TERRAIN.with(|c| c.set(value != "false"));
        } else if key == "quality" {
            if let Some(q) = QUALITIES.iter().position(|n| *n == value) {
                QUALITY.with(|c| c.set(q as u8));
            }
        } else if key == "selected" {
            if let Some(skill) = Skill::by_id(value) {
                SELECTED.with(|s| s.set(skill));
            }
        } else if let Some((id, part)) = key.rsplit_once('.')
            && let Some(skill) = Skill::by_id(id)
        {
            let parsed = if value == "infinite" { Some(f32::INFINITY) } else { value.parse::<f32>().ok() };
            let Some(v) = parsed else { continue };
            let mut t = tune(skill);
            match part {
                "power" => t.power = v,
                "damage" => t.damage = v,
                "speed" => t.speed = v,
                _ => continue,
            }
            set_tune(skill, t);
        }
    }
}

fn save(game: &super::Game<'_>) {
    let path = settings_path(game);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut out = String::from("# The Shooting Star (Demo): the remote's skills\n");
    out.push_str(&format!("cutscenes={}\n", CUTSCENES.with(std::cell::Cell::get)));
    out.push_str(&format!("terrain={}\n", TERRAIN.with(std::cell::Cell::get)));
    out.push_str(&format!("selected={}\n", selected().id()));
    out.push_str(&format!("quality={}\n", QUALITIES[usize::from(quality())]));
    for skill in Skill::ALL {
        let t = tune(skill);
        let damage = if t.damage.is_infinite() { "infinite".to_owned() } else { t.damage.to_string() };
        out.push_str(&format!("{0}.power={1}\n{0}.damage={2}\n{0}.speed={3}\n", skill.id(), t.power, damage, t.speed));
    }
    if let Err(error) = std::fs::write(&path, out) {
        log::warn!("[{ID}] the remote's settings could not be saved: {error}");
    }
}

/// The effects' quality settings, by name.
pub const QUALITIES: [&str; 4] = ["low", "medium", "high", "ultra"];

pub fn quality() -> u8 {
    QUALITY.with(std::cell::Cell::get).min(3)
}

/// Whether the held stack is the Stellar Remote.
pub fn is_remote(item: &str) -> bool {
    item == REMOTE_ID
}

fn holds_remote(game: &super::Game<'_>) -> bool {
    game.native_held().is_some_and(|(item, _, _)| is_remote(&item))
}

fn camera(game: &super::Game<'_>) -> Option<[f64; 3]> {
    game.native_player().map(|p| [p.x, p.y + 1.62, p.z])
}

// ---- keys -------------------------------------------------------------------------------------------------------

/// The key a spell answers to.
pub fn skill_key(game: &super::Game<'_>, skill: Skill) -> String {
    game.native_mod_key(ID, skill.id(), skill.default_key())
}

fn mod_key(game: &super::Game<'_>, name: &str) -> String {
    let default = KEYS.iter().find(|(n, _, _)| *n == name).map_or("key.keyboard.unknown", |(_, _, d)| *d);
    game.native_mod_key(ID, name, default)
}

/// How a key is written to the player: `key.keyboard.j` as `J`.
pub fn key_label(key: &str) -> String {
    let name = key.strip_prefix("key.keyboard.").unwrap_or(key);
    match name {
        "unknown" | "" => "No key".to_owned(),
        "space" => "Space".to_owned(),
        "backspace" => "Backspace".to_owned(),
        "escape" => "Esc".to_owned(),
        "enter" => "Enter".to_owned(),
        "tab" => "Tab".to_owned(),
        "left.shift" => "Left Shift".to_owned(),
        "left.control" => "Left Ctrl".to_owned(),
        "left.alt" => "Left Alt".to_owned(),
        other if other.chars().count() == 1 => other.to_uppercase(),
        other => other.replace('.', " "),
    }
}

/// The `key` hook: the remote's keys, as the original's `KeyMapping`s.
pub fn on_key(game: &mut super::Game<'_>, key: &str, pressed: bool) {
    if key == "key.keyboard.left.shift" || key == "key.keyboard.right.shift" {
        SHIFT.with(|s| s.set(pressed));
        return;
    }
    if !pressed {
        return;
    }
    load(game);
    // A menu row waiting for its key takes this press.
    if let Some(index) = BINDING.with(std::cell::Cell::get) {
        BINDING.with(|b| b.set(None));
        let Some(skill) = Skill::by_index(index) else { return };
        if key == "key.keyboard.escape" {
            return;
        }
        let cleared = key == "key.keyboard.backspace";
        game.native_set_mod_key(ID, skill.id(), Some(if cleared { "key.keyboard.unknown" } else { key }));
        ui_sound(if cleared { "ws_deny" } else { "ws_bound" }, 1.0);
        let said = if cleared { "No key · right-click to set one".to_owned() } else { format!("Selected by {}", key_label(key)) };
        game.native_client_action_bar(crate::text::Text::literal(said), true);
        return;
    }
    let menu = menu_open(game);
    if !menu && !game.native_playing() {
        return;
    }
    if menu && menu_key(game, key) {
        return;
    }
    if mod_key(game, "skill_menu") == key {
        toggle_menu(game);
        return;
    }
    if mod_key(game, "cutscenes") == key {
        let said = if skip_film() {
            None
        } else {
            let on = !CUTSCENES.with(std::cell::Cell::get);
            CUTSCENES.with(|c| c.set(on));
            save(game);
            Some(format!("Cutscenes: {}", if on { "On" } else { "Off" }))
        };
        if let Some(said) = said {
            game.native_client_action_bar(crate::text::Text::literal(said), true);
        }
        return;
    }
    // The rest act on the remote: in the hand, or with its menu open.
    if !menu && !holds_remote(game) {
        return;
    }
    let down = SHIFT.with(std::cell::Cell::get);
    let dir = if down { -1 } else { 1 };
    let skill = selected();
    if mod_key(game, "cycle") == key {
        select(game, skill.next());
        return;
    }
    if mod_key(game, "power") == key && skill.power().min < skill.power().max {
        let mut t = tune(skill);
        t.power = skill.power().step(t.power, dir);
        set_tune(skill, t);
        save(game);
        game.native_client_action_bar(crate::text::Text::literal(format!("{}: {}", skill.title(), skill.tune_line(t.power))), true);
        return;
    }
    if mod_key(game, "speed") == key {
        let mut t = tune(skill);
        t.speed = sk::SPEED.step(t.speed, dir);
        set_tune(skill, t);
        save(game);
        game.native_client_action_bar(crate::text::Text::literal(format!("{}: {}", skill.title(), speed_line(t.speed))), true);
        return;
    }
    if mod_key(game, "damage") == key {
        let mut t = tune(skill);
        let n = sk::DAMAGE_STEPS.len() as i32;
        let at = (sk::damage_step(t.damage) as i32 + dir).rem_euclid(n);
        t.damage = sk::DAMAGE_STEPS[at as usize];
        set_tune(skill, t);
        save(game);
        game.native_client_action_bar(crate::text::Text::literal(format!("{}: {}", skill.title(), damage_line(t.damage))), true);
        return;
    }
    let matches: Vec<Skill> = Skill::ALL.iter().copied().filter(|s| skill_key(game, *s) == key).collect();
    if let Some(first) = matches.first().copied() {
        select(game, first);
        if matches.len() > 1 {
            let others: Vec<&str> = matches[1..].iter().map(|s| s.title()).collect();
            game.native_client_action_bar(crate::text::Text::literal(format!("Also fires: {}", others.join(", "))), true);
        }
    }
}

pub fn damage_line(damage: f32) -> String {
    if damage.is_infinite() {
        "no limit: whatever it hits is erased".to_owned()
    } else {
        format!("{damage:.0} damage a hit ({:.1} hearts)", damage / 2.0)
    }
}

pub fn speed_line(speed: f32) -> String {
    format!("{speed:.2}x speed: the whole skill, its film too, plays faster or slower")
}

// ---- what the server says ---------------------------------------------------------------------------------------

/// One spell as the window plays it: the original's `SpellFx`.
#[derive(Clone, Debug)]
pub struct ClientSpell {
    pub start: sk::Start,
    pub age: i32,
    pub clock: f64,
    /// The age the player skipped its film at.
    pub skip_age: Option<i32>,
    pub pushed: bool,
    pub boomed: Option<i32>,
}

/// A jolt of the camera (`FxManager.shake`).
#[derive(Clone, Copy, Debug)]
struct Jolt {
    at: [f64; 3],
    strength: f32,
    reach: f64,
    born: u64,
}

/// The `net_from_server` hook.
pub fn on_message(channel: &str, data: &[u8]) {
    if channel != CHANNEL || data.is_empty() {
        return;
    }
    match data[0] {
        sk::FX_START => {
            if let Some(start) = sk::decode_start(data) {
                SPELLS.with(|s| {
                    let mut s = s.borrow_mut();
                    if s.len() >= 8 {
                        s.remove(0);
                    }
                    s.push(ClientSpell { start, age: 0, clock: 0.0, skip_age: None, pushed: false, boomed: None });
                    if start.mine && TRAILER.with(std::cell::Cell::get) {
                        MARKS.with(|m| m.borrow_mut().push(("cast", Some(start.skill), 0)));
                    }
                });
            }
        }
        sk::FX_STATE => {
            if let Some(state) = sk::decode_state(data) {
                COOLDOWNS.with(|c| *c.borrow_mut() = state);
            }
        }
        sk::FX_SELECT if data.len() >= 2 => {
            if let Some(skill) = Skill::by_index(data[1]) {
                SELECTED.with(|s| s.set(skill));
                SHOWN.with(|s| s.set(Some(std::time::Instant::now())));
            }
        }
        sk::FX_HELLO => {
            DIRTY.with(|d| d.set(true));
            // A world joined: what it holds of Imaginary Space follows.
            VOID.with(|v| v.borrow_mut().clear());
        }
        sk::FX_TRAILER if data.len() >= 2 => {
            let on = data[1] != 0;
            TRAILER.with(|t| t.set(on));
            if on {
                CUTSCENES.with(|c| c.set(true));
            }
            MARKS.with(|m| m.borrow_mut().push((if on { "start" } else { "end" }, None, 0)));
        }
        sk::FX_VOID if data.len() > 5 => {
            let seed = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
            if let Ok(dimension) = std::str::from_utf8(&data[5..]) {
                VOID.with(|v| v.borrow_mut().insert(dimension.to_owned(), seed));
            }
        }
        _ => {}
    }
}

thread_local! {
    /// The original's files are in the render pass's hands.
    static ASSETS_OK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ASSETS_TRIED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The first run's last word, said once.
    static SETUP_SAID: std::cell::RefCell<Option<super::shooting_star_setup::Status>> = const { std::cell::RefCell::new(None) };
}

/// The game folder (`horizon-mods/` is in it) and the mod's own data folder.
fn folders(game: &super::Game<'_>) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let data = game.native_data_dir(ID);
    let game_dir = data.parent()?.parent()?.parent()?.to_path_buf();
    Some((game_dir, data))
}

/// The original's files, from the content mod (the game's resources, or its
/// folder when the first run has only just made it), to the render pass.
fn read_assets(game: &super::Game<'_>, game_dir: Option<&std::path::Path>) -> usize {
    use horizon_client_render::gfx::native_render::shooting_star_fxframe as fx;
    let mut missing = 0;
    for path in fx::ASSET_FILES {
        let bytes = game.native_data(&format!("data/shooting_star_demo/{path}")).or_else(|| {
            let dir = super::shooting_star_setup::content_dir(game_dir?);
            std::fs::read(dir.join("data").join("shooting_star_demo").join(path)).ok()
        });
        match bytes {
            Some(bytes) => fx::provide(path, bytes),
            None => missing += 1,
        }
    }
    missing
}

/// Once a tick: the original's files read, or the first run started and
/// followed (it downloads the original and builds the content mod).
fn load_assets(game: &mut super::Game<'_>) {
    use super::shooting_star_setup as setup;
    let dirs = folders(game);
    if !ASSETS_TRIED.with(|t| t.replace(true)) {
        let missing = read_assets(game, dirs.as_ref().map(|d| d.0.as_path()));
        // The shaders and textures make the light; the films are a part of it.
        let core = horizon_client_render::gfx::native_render::shooting_star_fxframe::asset("fx/composite.fsh").is_some();
        ASSETS_OK.with(|ok| ok.set(core));
        if let Some((game_dir, data)) = &dirs {
            if !setup::ready(game_dir) {
                log::info!("[{ID}] the content mod is not built yet: the first run starts");
                setup::start(game_dir.clone(), data.join("original"), false);
            } else {
                if setup::turn_on(game_dir).unwrap_or(false) {
                    say(game, "The Shooting Star: its content mod was off and is turned on again. Restart the game.");
                }
                if !setup::films_ready(game_dir) {
                    setup::start(game_dir.clone(), data.join("original"), true);
                }
            }
        }
        if missing == 0 {
            log::info!("[{ID}] the original's {} files are read", horizon_client_render::gfx::native_render::shooting_star_fxframe::ASSET_FILES.len());
        }
    }
    let status = setup::status();
    let said = SETUP_SAID.with(|s| s.borrow().clone());
    if said.as_ref() == Some(&status) || status == setup::Status::Idle {
        return;
    }
    match &status {
        setup::Status::Working(what) => {
            if TICKS.with(std::cell::Cell::get) % 40 == 0 {
                game.native_client_action_bar(crate::text::Text::literal(format!("✦ The Shooting Star: {what}…")), false);
            }
            return;
        }
        // The last word waits for a world, where it can be read.
        setup::Status::Done { .. } | setup::Status::Failed(_) if game.native_player().is_none() => return,
        setup::Status::Done { films } => {
            let missing = read_assets(game, dirs.as_ref().map(|d| d.0.as_path()));
            ASSETS_OK.with(|ok| ok.set(missing == 0 || *films || horizon_client_render::gfx::native_render::shooting_star_fxframe::asset("fx/composite.fsh").is_some()));
            let films_note = if *films { "" } else { " The films need Java 25 (Horizon Launcher installs it with any 26.x instance)." };
            say(game, &format!("✦ The Shooting Star is ready: the light works now; restart the game for the Stellar Remote, its blocks and sounds.{films_note}"));
        }
        setup::Status::Failed(error) => {
            say(game, &format!("✦ The Shooting Star could not get the original's files: {error}. It tries again next start."));
        }
        setup::Status::Idle => {}
    }
    SETUP_SAID.with(|s| *s.borrow_mut() = Some(status));
}

/// A line in the chat, for this player only.
fn say(game: &mut super::Game<'_>, text: &str) {
    game.native_client_action_bar(crate::text::Text::literal(text.to_owned()), false);
    game.native_chat(text);
}

/// The light can be drawn: the original's shaders are in.
pub fn assets_ok() -> bool {
    ASSETS_OK.with(std::cell::Cell::get)
}

/// The window's tick (`tick`): the selection reaches the server, the
/// recharges count down and every spell's timeline moves on.
pub fn client_tick(game: &mut super::Game<'_>) {
    load(game);
    play_ui_sounds(game);
    load_assets(game);
    write_marks(game);
    let now = TICKS.with(|t| {
        t.set(t.get() + 1);
        t.get()
    });
    if let Some(index) = PENDING.with(std::cell::Cell::take)
        && let Some(skill) = Skill::by_index(index)
    {
        select(game, skill);
    }
    if DIRTY.with(std::cell::Cell::get) || now % 100 == 0 {
        let tunes = TUNES.with(|t| *t.borrow());
        let a = game.native_net_send_to_server(CH_CMD, &[sk::CMD_SELECT, selected().index()]);
        let b = game.native_net_send_to_server(CH_CMD, &sk::encode_tunes(&tunes));
        let flags = u8::from(TERRAIN.with(std::cell::Cell::get));
        let c = game.native_net_send_to_server(CH_CMD, &[sk::CMD_FLAGS, flags]);
        if a.is_ok() && b.is_ok() && c.is_ok() {
            DIRTY.with(|d| d.set(false));
        }
    }
    COOLDOWNS.with(|c| {
        for slot in c.borrow_mut().iter_mut() {
            slot.0 = slot.0.saturating_sub(1);
        }
    });
    SHAKES.with(|s| s.borrow_mut().retain(|j| now.saturating_sub(j.born) < 60));
    let mut spells = SPELLS.with(|s| std::mem::take(&mut *s.borrow_mut()));
    let (bottom, height) = game.native_world_height();
    direct(&spells, (bottom, bottom + height - 1));
    for spell in spells.iter_mut() {
        let until = (spell.clock + 1.0e-6).floor() as i32;
        while spell.age <= until && spell.age <= spell.start.skill.fx_duration() {
            let t = spell.age;
            cue(game, spell, t, now);
            spell.age += 1;
        }
        spell.clock += f64::from(spell.start.speed);
        shockwave(game, spell);
        pull(game, spell);
        nova_wave(game, spell, now);
    }
    spells.retain(|s| s.age <= s.start.skill.fx_duration());
    SPELLS.with(|s| {
        let mut s = s.borrow_mut();
        spells.append(&mut s);
        *s = spells;
    });
}

/// How far a spell's own zone reaches round its target.
fn zone_of(start: &sk::Start) -> f64 {
    match start.skill {
        Skill::ShootingStar => f64::from(start.power),
        Skill::Gungnir => sk::gungnir_radius(start.power),
        Skill::SevenStars => sk::dipper_span(f64::from(start.power)) * 0.5,
        Skill::Tesseract => f64::from(sk::tess_zone(sk::tess_scale(start.power))) * 0.5,
        Skill::Gravitia => 1.0e7,
        Skill::EventHorizon => sk::eh_radius(start.power),
        Skill::Supernova => sk::sn_radius(start.power),
    }
}

// ---- the films ----------------------------------------------------------------------------------------------------

use super::shooting_star_cutscenes as cs;

/// The film playing: the newest of this player's own spells that has one,
/// as the original's `CutsceneDirector` keeps one `current`.
struct Playing {
    seed: u64,
    film: cs::Cutscene,
}

thread_local! {
    static PLAYING: std::cell::RefCell<Option<Playing>> = const { std::cell::RefCell::new(None) };
    /// What the film did to the camera this frame: its weight, lens and roll.
    static LENS: std::cell::Cell<(f32, f32, f32)> = const { std::cell::Cell::new((0.0, 70.0, 0.0)) };
}

/// The film's weight at film tick `t` for `spell`, if it is the one playing.
fn film_weight_at(spell: &ClientSpell, t: f32) -> Option<f32> {
    if !spell.start.mine || !CUTSCENES.with(std::cell::Cell::get) {
        return None;
    }
    PLAYING.with(|p| {
        let p = p.borrow();
        let p = p.as_ref().filter(|p| p.seed == spell.start.seed)?;
        let skipped = spell.skip_age.map(|at| (spell.age - at) as f32);
        let w = p.film.weight(t, skipped);
        (w > 0.001).then_some(w)
    })
}

/// The film holds this spell's sound and screen (`inFilm`: over half its weight).
fn film_on(spell: &ClientSpell) -> bool {
    film_weight_at(spell, spell.age as f32).is_some_and(|w| w > 0.5)
}

// ---- the trailer ----------------------------------------------------------------------------------------------
//
// `/tss trailer` runs a director on the server (`shooting_star_spells`): it
// casts every skill in turn, each from a fresh spot. Here the window hides
// its HUD, plays every film, and writes when each moment happened into
// `shooting_star_trailer.jsonl` in the game folder, which the recording
// script (`tools/trailer/record.ps1`) cuts the footage by.

thread_local! {
    static TRAILER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Moments to write: what, which skill, its age.
    static MARKS: std::cell::RefCell<Vec<(&'static str, Option<Skill>, i32)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The director is filming: no HUD, no hand.
pub fn trailer() -> bool {
    TRAILER.with(std::cell::Cell::get)
}

fn write_marks(game: &super::Game<'_>) {
    if TRAILER.with(std::cell::Cell::get) {
        // Every own spell's age, each half second, for the cuts to land on its ticks.
        SPELLS.with(|s| {
            for spell in s.borrow().iter().filter(|s| s.start.mine && s.age % 10 == 0) {
                MARKS.with(|m| m.borrow_mut().push(("age", Some(spell.start.skill), spell.age)));
            }
        });
    }
    let marks = MARKS.with(|m| std::mem::take(&mut *m.borrow_mut()));
    if marks.is_empty() {
        return;
    }
    let Some((game_dir, _)) = folders(game) else { return };
    let path = game_dir.join("shooting_star_trailer.jsonl");
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let mut text = String::new();
    for (event, skill, age) in &marks {
        let skill = skill.map_or("", |s| s.id());
        text.push_str(&format!("{{\"event\":\"{event}\",\"skill\":\"{skill}\",\"age\":{age},\"ms\":{ms}}}\n"));
    }
    use std::io::Write;
    let fresh = marks.first().is_some_and(|(e, _, _)| *e == "start");
    let file = std::fs::OpenOptions::new().create(true).write(true).append(!fresh).truncate(fresh).open(&path);
    match file {
        Ok(mut f) => {
            let _ = f.write_all(text.as_bytes());
        }
        Err(error) => log::warn!("[{ID}] the trailer's marks could not be written: {error}"),
    }
}

/// How much a film holds the camera this frame.
pub fn film_weight() -> f32 {
    LENS.with(std::cell::Cell::get).0
}

/// Picks the film to play, each tick: a new spell of this player's starts
/// its own, the last one's ends with it.
fn direct(spells: &[ClientSpell], heights: (i32, i32)) {
    let newest = spells.iter().rev().find(|s| s.start.mine);
    PLAYING.with(|p| {
        let mut p = p.borrow_mut();
        match newest {
            Some(spell) if p.as_ref().is_none_or(|p| p.seed != spell.start.seed) => {
                let nodes = (spell.start.skill == Skill::SevenStars).then(|| super::shooting_star_visuals::dipper_nodes(&spell.start));
                if let Some(film) = cs::of(&spell.start, nodes, heights) {
                    *p = Some(Playing { seed: spell.start.seed, film });
                }
            }
            Some(_) => {}
            None => *p = None,
        }
        if let Some(playing) = p.as_ref()
            && let Some(spell) = spells.iter().find(|s| s.start.seed == playing.seed)
            && playing.film.finished(spell.age as f32)
        {
            *p = None;
        }
    });
}

/// Skip the film playing, as the cutscene key does.
fn skip_film() -> bool {
    SPELLS.with(|s| {
        let mut skipped = false;
        for spell in s.borrow_mut().iter_mut() {
            if spell.skip_age.is_none() && film_weight_at(spell, spell.age as f32).is_some() {
                spell.skip_age = Some(spell.age);
                skipped = true;
            }
        }
        if skipped {
            log::info!("[{ID}] the film is skipped");
        }
        skipped
    })
}

/// The ground and walls of the window's world, for the film's camera.
struct WorldGround<'a, 'g> {
    game: &'a super::Game<'g>,
    bottom: i32,
    top: i32,
}

impl WorldGround<'_, '_> {
    fn blocks(&self, x: i32, y: i32, z: i32) -> bool {
        if !self.game.native_block_collision([x, y, z]).is_empty() {
            return true;
        }
        let name = self.game.native_block([x, y, z]);
        name.ends_with("water") || name.ends_with("lava")
    }
}

impl cs::Ground for WorldGround<'_, '_> {
    fn top(&self, x: f64, z: f64) -> Option<f64> {
        let (bx, bz) = (x.floor() as i32, z.floor() as i32);
        (self.bottom..=self.top).rev().find(|y| self.blocks(bx, *y, bz)).map(|y| f64::from(y + 1))
    }
    fn solid(&self, at: cs::V) -> bool {
        let y = at[1].floor() as i32;
        if y < self.bottom {
            return true;
        }
        y <= self.top && !self.game.native_block_collision([at[0].floor() as i32, y, at[2].floor() as i32]).is_empty()
    }
}

/// The `camera_pose` hook: while a film plays, its shot, blended with the
/// player's own view by the film's weight.
pub fn camera_pose(game: &super::Game<'_>, partial: f32, position: &mut [f64; 3], yaw: &mut f32, pitch: &mut f32) {
    LENS.with(|l| l.set((0.0, 70.0, 0.0)));
    let Some(player) = game.native_player() else { return };
    let (bottom, height) = game.native_world_height();
    let ground = WorldGround { game, bottom, top: bottom + height - 1 };
    let pose = SPELLS.with(|s| {
        let s = s.borrow();
        let spell = s.iter().rev().find(|spell| film_weight_at(spell, spell_time(spell, partial)).is_some())?;
        let t = spell_time(spell, partial);
        let weight = film_weight_at(spell, t)?;
        let yaw0 = f64::from(spell.start.yaw).to_radians();
        let subject = cs::Subject { feet: [player.x, player.y, player.z], target: spell.start.target, forward: [-yaw0.sin(), 0.0, yaw0.cos()] };
        PLAYING.with(|p| p.borrow().as_ref().and_then(|p| p.film.pose(t, &subject, &ground))).map(|shot| (shot, weight))
    });
    let Some((shot, weight)) = pose else { return };
    let blended = cs::blend(*position, *yaw, *pitch, &shot, weight);
    let (y, p) = (blended.yaw(), blended.pitch());
    if blended.position.iter().all(|v| v.is_finite()) && y.is_finite() && p.is_finite() {
        *position = blended.position;
        *yaw = y;
        *pitch = p;
        LENS.with(|l| l.set((weight, shot.fov, shot.roll)));
    }
}

/// The `compute_fov` hook: the shot's lens, by the film's weight.
pub fn compute_fov(degrees: &mut f32) {
    let (weight, fov, _) = LENS.with(std::cell::Cell::get);
    if weight > 0.001 {
        *degrees += (fov - *degrees) * weight;
    }
}

/// Where a far spell is heard: the edge of its zone nearest the listener,
/// as the original's `edgeToward`.
fn ear(cam: [f64; 3], target: [f64; 3], zone: f64) -> [f64; 3] {
    let dx = cam[0] - target[0];
    let dz = cam[2] - target[2];
    let d = dx.hypot(dz);
    if d <= zone || d < 1e-6 {
        cam
    } else {
        [target[0] + dx / d * zone, cam[1], target[2] + dz / d * zone]
    }
}

thread_local! {
    /// The spell sounding now is in its film: its sounds are in the head, as the original's are then.
    static IN_FILM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The original's `sound(...)` out in the world: its volume times 16, so it
/// carries 256 blocks (16 times the event's own reach), loud up to the ear.
fn play(game: &mut super::Game<'_>, name: &str, at: [f64; 3], volume: f32, pitch: f32) {
    if IN_FILM.with(std::cell::Cell::get) {
        ears(game, name, volume);
        return;
    }
    game.native_play_local_sound(&format!("shooting_star_demo:{name}"), at, volume * 16.0, pitch, 0);
}

/// The original's `cue(...)` (`ScoreCue`): volume one, so it carries as far as
/// its own `sounds.json` entry says (256 blocks for the railgun's).
fn cue_at(game: &mut super::Game<'_>, film: bool, name: &str, at: [f64; 3]) {
    if film {
        ears(game, name, 1.0);
        return;
    }
    game.native_play_local_sound(&format!("shooting_star_demo:{name}"), at, 1.0, 1.0, 0);
}

/// The original's `ears(...)` and film sounds: heard as if inside the head,
/// wherever the camera is (the film's camera carries the listener hundreds of
/// blocks off): a sound with no place and no fall-off, as the original's
/// relative `Attenuation.NONE` ones.
fn ears(game: &mut super::Game<'_>, name: &str, volume: f32) {
    if !game.native_window() {
        return;
    }
    game.audio.play(&format!("shooting_star_demo:{name}"), None, volume.clamp(0.0, 1.0), 1.0);
}

/// `sound(..., film)`: in the head during the film, out in the world else.
fn sound(game: &mut super::Game<'_>, film: bool, name: &str, at: [f64; 3], volume: f32, pitch: f32) {
    if film { ears(game, name, volume) } else { play(game, name, at, volume, pitch) }
}

/// Gravitia's score is mastered hotter than the others': this brings it level with them.
const GRAV_VOLUME: f32 = 0.35;

fn jolt(at: [f64; 3], strength: f32, reach: f64, now: u64) {
    SHAKES.with(|s| s.borrow_mut().push(Jolt { at, strength, reach, born: now }));
}

/// One tick of a spell's sound and jolts: the original's `StarFx`,
/// `GungnirFx`, `SevenStarsFx`, the Tesseract's score and Gravitia's.
fn cue(game: &mut super::Game<'_>, spell: &mut ClientSpell, t: i32, now: u64) {
    let Some(cam) = camera(game) else { return };
    let start = spell.start;
    let film = film_on(spell);
    IN_FILM.with(|f| f.set(film));
    let zone = zone_of(&start);
    let edge = if film { cam } else { ear(cam, start.target, zone) };
    let caster = start.origin;
    let target = start.target;
    let far = (cam[0] - target[0]).hypot(cam[2] - target[2]);
    match start.skill {
        Skill::ShootingStar => {
            match t {
                0 => cue_at(game, film, "star_rg_arm", caster),
                7 => cue_at(game, film, "star_rg_press", caster),
                9 if film => ears(game, "star_lock", 1.0),
                9 => cue_at(game, film, "star_lock", edge),
                29 if !film => play(game, "star_countdown", edge, 1.0, 1.0),
                44 if film => ears(game, "star_ascent", 1.0),
                238 if film => ears(game, "star_gun", 1.0),
                314 if film => ears(game, "star_fire", 1.0),
                352 => {
                    if film { ears(game, "star_impact", 1.0) } else { play(game, "star_impact", edge, 1.0, 1.0) }
                    cue_at(game, film, "star_beam", edge);
                    jolt(edge, 1.0, 700.0, now);
                }
                462 => cue_at(game, film, "star_rg_collapse", edge),
                490 => cue_at(game, film, "star_rg_after", edge),
                _ => {}
            }
            if t > 312 && t < 352 && t % 4 == 0 {
                let k = (t - 352 + 40) as f32 / 40.0;
                jolt(edge, 0.05 + 0.15 * k * k, 400.0, now);
            }
        }
        Skill::Gungnir => {
            let radius = sk::gungnir_radius(start.power);
            let near = far < 2000.0;
            match t {
                0 => play(game, "star_arm", caster, 0.8, 1.0),
                7 => play(game, "star_press", caster, 0.9, 1.0),
                9 => sound(game, film, "gungnir_mark", edge, 1.0, 1.0),
                29 if !film => play(game, "gungnir_countdown", edge, 1.0, 1.0),
                44 if film => ears(game, "gungnir_ascent", 1.0),
                124 if film => ears(game, "gungnir_ring", 1.0),
                146 if film => ears(game, "gungnir_load", 1.0),
                168 if film => ears(game, "gungnir_spin", 1.0),
                247 if film => ears(game, "gungnir_release", 1.0),
                256 if film => ears(game, "gungnir_flight", 1.0),
                290 if film || near => {
                    let v = if film { 1.0 } else { (1.2 - far / 2000.0).clamp(0.3, 1.0) as f32 };
                    ears(game, "gungnir_hush", v);
                }
                322 if film || near => ears(game, "gungnir_crack", if film { 1.0 } else { 0.8 }),
                372 if film => ears(game, "gungnir_hum", 1.0),
                372 => play(game, "gungnir_hum", target, 0.9, 1.0),
                _ => {}
            }
            // The boom arrives with the shock front, as far as it carries.
            let boom_at = 322.0 + 36.0 * (far.max(0.0) / radius.max(1.0)).powf(1.666_666_7);
            if spell.boomed.is_none() && t >= 322 && f64::from(t) >= boom_at && far < 4000.0 {
                spell.boomed = Some(t);
                let loud = (1.2 - far / 3000.0).clamp(0.25, 1.0) as f32;
                ears(game, "gungnir_boom", if film { 1.0 } else { loud });
                jolt(cam, 2.4 * loud, 4000.0, now);
            }
            if spell.boomed.is_some_and(|b| t == b + 2) {
                let v = if film { 1.0 } else { (1.2 - far / 900.0).clamp(0.2, 1.0) as f32 };
                ears(game, "gungnir_wind", v);
            }
        }
        Skill::SevenStars => {
            let scale = f64::from(start.power);
            let k = scale.max(1.0) as f32;
            match t {
                0 => play(game, "star_arm", caster, 0.8, 1.0),
                7 => play(game, "star_press", caster, 0.9, 1.0),
                9 => sound(game, film, "stars_mark", edge, 1.0, 1.0),
                24 => sound(game, film, "stars_lock", edge, 1.0, 1.0),
                29 if !film => play(game, "stars_countdown", edge, 1.0, 1.0),
                56 if film => ears(game, "stars_ascent", 1.0),
                130 if film => ears(game, "stars_array", 1.0),
                208 if film => ears(game, "stars_fire", 1.0),
                236 if film => ears(game, "stars_return", 1.0),
                398 => play(game, "stars_link", edge, 1.0, 1.0),
                434 => play(game, "stars_finale", edge, 1.0, 1.0),
                440 => jolt(target, 1.8, 1600.0 * scale, now),
                470 => play(game, "stars_hum", edge, 0.9, 1.0),
                _ => {}
            }
            let turn = sk::dipper_turn(start.origin[0], start.origin[2], target[0], target[2]);
            for i in 0..7 {
                let land = sk::dipper_land_tick(i);
                let n = sk::dipper_node(i, turn, scale);
                let node = [target[0] + n[0], target[1], target[2] + n[1]];
                let pitch = 0.85 + i as f32 * 0.05;
                if t == land - 18 {
                    sound(game, film, "stars_fall", node, if film { 0.6 } else { 0.6 * k }, pitch);
                }
                if t == land {
                    sound(game, film, "stars_land", node, if film { 1.0 } else { k }, pitch);
                    jolt(node, 1.1, 900.0 * scale, now);
                }
            }
        }
        Skill::Tesseract => {
            const SCORE: [i32; 16] = [0, 55, 148, 207, 288, 333, 353, 397, 423, 456, 470, 491, 508, 527, 554, 586];
            if let Some(n) = SCORE.iter().position(|at| *at == t) {
                let e = ear(cam, target, zone);
                let d = if film { 0.0 } else { ((e[0] - cam[0]).powi(2) + (e[1] - cam[1]).powi(2) + (e[2] - cam[2]).powi(2)).sqrt() };
                let v = ((2800.0 - d) / 1400.0).clamp(0.0, 1.0) as f32;
                if v > 0.0 {
                    ears(game, &format!("tess_rw_{:02}", n + 1), v);
                }
            }
            if t == 462 {
                jolt(target, 1.4, 2000.0, now);
            }
            for ring in 0..4 {
                if t == sk::tess_wave_tick(ring) {
                    jolt(target, 0.8, 1600.0, now);
                }
            }
        }
        Skill::Gravitia => {
            // Its whole score is heard in the head, wherever the player stands.
            for (name, volume) in super::shooting_star_gravitia::sounds(t) {
                ears(game, &name, volume * GRAV_VOLUME);
            }
            if let Some(strength) = super::shooting_star_gravitia::jolts(t) {
                jolt(cam, strength as f32 * 2.0, 1.0e9, now);
            }
            // The machine firing hums through the ground.
            if (288..430).contains(&t) && t % 8 == 0 {
                jolt(cam, 0.25, 1.0e9, now);
            }
            let _ = caster;
        }
        Skill::EventHorizon => event_horizon_cue(game, &start, film, cam, t, now),
        Skill::Supernova => supernova_cue(game, &start, film, cam, t, now),
    }
}

/// The port's own score (`ss_port:*`, made by `tools/sounds/synth.py`):
/// heard in the head, as the original's Gravitia score is, louder the nearer
/// the camera is to `at` (full within `near` blocks, gone past `far`).
fn score(game: &mut super::Game<'_>, film: bool, name: &str, cam: [f64; 3], at: [f64; 3], near: f64, far: f64) {
    if !game.native_window() {
        return;
    }
    let d = ((cam[0] - at[0]).powi(2) + (cam[1] - at[1]).powi(2) + (cam[2] - at[2]).powi(2)).sqrt();
    let v = if film { 1.0 } else { (1.0 - (d - near) / (far - near).max(1.0)).clamp(0.0, 1.0) as f32 };
    if v > 0.01 {
        game.audio.play(&format!("ss_port:{name}"), None, v, 1.0);
    }
}

/// One of the port's own sounds out in the world, carrying `reach` blocks.
fn spot(game: &mut super::Game<'_>, name: &str, at: [f64; 3], reach: f32, pitch: f32) {
    game.native_play_local_sound(&format!("ss_port:{name}"), at, (reach / 16.0).max(1.0), pitch, 0);
}

/// SS-H1's sound: the remote arming, the satellite crushed, the fall, the
/// landing, the feeding swelling for nine seconds, the collapse cut off into
/// silence, and the flash.
fn event_horizon_cue(game: &mut super::Game<'_>, start: &sk::Start, film: bool, cam: [f64; 3], t: i32, now: u64) {
    let radius = sk::eh_radius(start.power);
    let hole = sk::eh_hole(start.target, start.power, f64::from(t));
    let centre = sk::eh_center(start.target, start.power);
    let (near, far) = (radius * 4.0 + 60.0, radius * 14.0 + 700.0);
    let name = match t {
        9 => Some("eh_arm"),
        40 => Some("eh_crush"),
        t if t == sk::EH_DROP => Some("eh_fall"),
        t if t == sk::EH_LAND => Some("eh_land"),
        150 => Some("eh_feed"),
        t if t == sk::EH_COLLAPSE => Some("eh_collapse"),
        t if t == sk::EH_FLASH => Some("eh_flash"),
        _ => None,
    };
    if let Some(name) = name {
        // The flash is heard from much further off.
        let far = if t == sk::EH_FLASH { far * 2.5 } else { far };
        score(game, film, name, cam, hole, near, far);
    }
    // The land torn away round the hole, here and there.
    if (sk::EH_FEED..sk::EH_COLLAPSE).contains(&t) && (t * 7 + (start.seed & 15) as i32) % 23 == 0 {
        let a = (t as f64 * 2.399 + start.seed as f64).sin() * std::f64::consts::PI;
        let r = radius * sk::eh_eaten(f64::from(t) + 10.0);
        let at = [centre[0] + a.cos() * r, centre[1], centre[2] + a.sin() * r];
        spot(game, "eh_tear", at, (radius * 6.0 + 96.0) as f32, 0.85 + 0.3 * ((t % 5) as f32 / 4.0));
    }
    if t == sk::EH_FLASH + 30 || t == sk::EH_FLASH + 200 {
        spot(game, "eh_rim", centre, (radius * 4.0 + 64.0) as f32, 1.0);
    }
    if t == sk::EH_LAND {
        jolt(hole, 1.2, radius * 10.0 + 300.0, now);
    }
    if (sk::EH_FEED..sk::EH_COLLAPSE).contains(&t) && t % 10 == 0 {
        jolt(hole, 0.25 + 0.35 * sk::eh_eaten(f64::from(t)) as f32, radius * 6.0 + 200.0, now);
    }
    if t == sk::EH_FLASH {
        jolt(hole, 2.4, radius * 16.0 + 600.0, now);
    }
}

/// SS-H2's sound: the remote arming, the star lit, its nine-second swell, the
/// fall into silence, the blast, and the dawn after.
fn supernova_cue(game: &mut super::Game<'_>, start: &sk::Start, film: bool, cam: [f64; 3], t: i32, now: u64) {
    let radius = sk::sn_radius(start.power);
    let star = sk::sn_star(start.target);
    let (near, far) = (sk::SN_HEIGHT + radius, sk::SN_HEIGHT * 3.0 + radius * 4.0 + 1200.0);
    let name = match t {
        9 => Some("sn_arm"),
        t if t == sk::SN_LIGHT => Some("sn_ignite"),
        t if t == sk::SN_SWELL + 2 => Some("sn_swell"),
        t if t == sk::SN_PEAK => Some("sn_implode"),
        t if t == sk::SN_BLAST => Some("sn_blast"),
        t if t == sk::SN_BLAST + 50 => Some("sn_dawn"),
        _ => None,
    };
    if let Some(name) = name {
        let far = if t == sk::SN_BLAST { far * 2.0 } else { far };
        score(game, film, name, cam, star, near, far);
    }
    if t == sk::SN_HIT + 12 {
        spot(game, "sn_burn", start.target, (radius * 1.5 + 96.0) as f32, 1.0);
    }
    // The star's pulse throbs through the ground as it swells.
    if (sk::SN_SWELL..sk::SN_PEAK).contains(&t) && t % 25 == 0 {
        jolt(cam, 0.15 + 0.35 * (t - sk::SN_SWELL) as f32 / (sk::SN_PEAK - sk::SN_SWELL) as f32, 1.0e9, now);
    }
    if t == sk::SN_BLAST {
        jolt(cam, 1.2, 1.0e9, now);
    }
    if t == sk::SN_HIT {
        jolt(start.target, 2.0, radius * 3.0 + 200.0, now);
    }
}

/// The Supernova's ground front reaching the player: the wind and grit of it
/// going by, a jolt, and a shove outward and up.
fn nova_wave(game: &mut super::Game<'_>, spell: &mut ClientSpell, now: u64) {
    if spell.start.skill != Skill::Supernova || spell.pushed || spell.age < sk::SN_HIT {
        return;
    }
    let Some(player) = game.native_player() else { return };
    let radius = sk::sn_radius(spell.start.power);
    let (dx, dz) = spell.target_dx(player.x, player.z);
    let d = dx.hypot(dz);
    let front = sk::sn_ring(f64::from(spell.age), spell.start.power);
    if d > radius * 1.15 + 8.0 {
        spell.pushed = true;
        return;
    }
    if front + 2.0 < d && front < radius * 0.999 {
        return;
    }
    spell.pushed = true;
    let at = [player.x, player.y + 1.0, player.z];
    if game.native_window() {
        game.audio.play("ss_port:sn_wave", None, 1.0, 1.0);
    }
    jolt(at, 1.6, 64.0, now);
    if !spell.start.mine && d > 1.0 {
        let k = (1.0 - d / (radius * 1.15 + 8.0)).clamp(0.2, 1.0);
        game.native_push_player([(dx / d * 1.6 * k) as f32, (0.5 + 0.6 * k) as f32, (dz / d * 1.6 * k) as f32]);
    }
}

/// The hole pulls the player in while it feeds (the caster stands it out).
fn pull(game: &mut super::Game<'_>, spell: &ClientSpell) {
    if spell.start.skill != Skill::EventHorizon || spell.start.mine || !(sk::EH_LAND..sk::EH_FLASH).contains(&spell.age) {
        return;
    }
    let Some(player) = game.native_player() else { return };
    let hole = sk::eh_hole(spell.start.target, spell.start.power, f64::from(spell.age));
    let d = [hole[0] - player.x, hole[1] - (player.y + 1.0), hole[2] - player.z];
    let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let reach = sk::eh_pull(spell.start.power);
    let hunger = 0.4 + 0.6 * sk::eh_eaten(f64::from(spell.age)) as f64;
    if n < 1.0 || n > reach {
        return;
    }
    // Gentle at the edge of its reach, past running away from close in.
    let k = (0.025 + 0.42 * (1.0 - n / reach).powi(2)) * hunger * if spell.age >= sk::EH_FEED { 1.0 } else { 0.5 };
    game.native_push_player([(d[0] / n * k) as f32, (d[1] / n * k + 0.02) as f32, (d[2] / n * k) as f32]);
}

/// Gungnir's shock front throws the player outward when it passes them,
/// past the rim (the server hurts them, the window moves them).
fn shockwave(game: &mut super::Game<'_>, spell: &mut ClientSpell) {
    if spell.start.skill != Skill::Gungnir || spell.pushed || spell.age <= 322 {
        return;
    }
    let Some(player) = game.native_player() else { return };
    let radius = sk::gungnir_radius(spell.start.power);
    let t = spell.target_dx(player.x, player.z);
    let d = t.0.hypot(t.1);
    let front = sk::gungnir_front(f64::from(spell.age), radius);
    if d <= radius || d > radius * 2.5 {
        return;
    }
    if front >= d {
        spell.pushed = true;
        let k = 1.0 - (d - radius) / (radius * 1.5);
        let out = 1.2 + 2.8 * k;
        game.native_push_player([(t.0 / d * out) as f32, (0.6 + 1.2 * k) as f32, (t.1 / d * out) as f32]);
    }
}

impl ClientSpell {
    fn target_dx(&self, x: f64, z: f64) -> (f64, f64) {
        (x - self.start.target[0], z - self.start.target[2])
    }
}

/// The `camera_shake` hook: every jolt still ringing, weaker further off.
pub fn shake(game: &super::Game<'_>, partial: f32, shake: &mut super::NativeShake) {
    let (weight, _, roll) = LENS.with(std::cell::Cell::get);
    shake.roll += roll * weight;
    let Some(eye) = camera(game) else { return };
    let now = TICKS.with(std::cell::Cell::get) as f32 + partial;
    SHAKES.with(|s| {
        for j in s.borrow().iter() {
            let age = now - j.born as f32;
            if age < 0.0 || age > 50.0 {
                continue;
            }
            let d = ((eye[0] - j.at[0]).powi(2) + (eye[1] - j.at[1]).powi(2) + (eye[2] - j.at[2]).powi(2)).sqrt();
            if d > j.reach {
                continue;
            }
            let falloff = (1.0 - d / j.reach) as f32;
            let decay = (1.0 - age / 50.0).powi(2);
            let amplitude = 2.2 * j.strength * falloff * decay;
            let time = age * 0.8;
            shake.pitch += amplitude * time.sin();
            shake.yaw += amplitude * (time * 1.7).cos();
            shake.roll += amplitude * 0.6 * (time * 2.3).sin();
        }
    });
}

// ---- the spell menu -----------------------------------------------------------------------------------------------

/// Whether the mod's own screen is the one on the screen.
pub fn menu_open(game: &super::Game<'_>) -> bool {
    game.native_mod_screen() == Some(ID)
}

/// The menu key: open the spell menu, or close it.
pub fn toggle_menu(game: &mut super::Game<'_>) {
    if menu_open(game) {
        game.native_mod_screen_close(ID);
        return;
    }
    if game.native_mod_screen_open(ID) {
        if let Some(eye) = camera(game) {
            game.native_play_local_sound("shooting_star_demo:ws_open", eye, 0.8, 1.0, 0);
        }
    } else {
        game.native_client_action_bar(crate::text::Text::literal("The remote's menu cannot open here".to_owned()), true);
    }
}

// The menu is laid out once per frame from the screen's size: the same
// rectangles draw it and take its clicks.

/// One of the three tunings the menu sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Knob {
    Power,
    Damage,
    Speed,
}

const KNOBS: [Knob; 3] = [Knob::Power, Knob::Damage, Knob::Speed];

/// What a point of the menu is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spot {
    Skill(u8),
    Down(Knob),
    Up(Knob),
    Row(Knob),
    Key,
    Films,
    Reset,
    Quality,
    Terrain,
}

struct Menu {
    panel: [f32; 4],
    list: [[f32; 4]; sk::SKILLS],
    banner: [f32; 4],
    text_y: f32,
    rows: [[f32; 4]; 3],
    down: [[f32; 4]; 3],
    up: [[f32; 4]; 3],
    key: [f32; 4],
    films: [f32; 4],
    reset: [f32; 4],
    quality: [f32; 4],
    terrain: [f32; 4],
}

fn layout(width: f32, height: f32) -> Menu {
    let pw = 440.0_f32.min(width - 12.0).max(260.0);
    let ph = 272.0_f32.min(height - 12.0).max(200.0);
    let (x, y) = (((width - pw) / 2.0).floor(), ((height - ph) / 2.0).floor());
    let top = y + 26.0;
    let list_w = (pw * 0.36).floor();
    let n = sk::SKILLS as f32;
    let row_h = ((ph - 26.0 - 22.0) / n).min(42.0).floor();
    let mut list = [[0.0; 4]; sk::SKILLS];
    for (i, row) in list.iter_mut().enumerate() {
        *row = [x + 8.0, top + i as f32 * row_h, list_w, row_h - 4.0];
    }
    let rx = x + 8.0 + list_w + 8.0;
    let rw = x + pw - 8.0 - rx;
    // The picture takes what the words, tunings and buttons leave.
    let banner_h = (rw / 2.0).min(92.0).min(ph - 150.0).max(30.0).floor();
    let banner = [rx, top, rw, banner_h];
    let text_y = top + banner_h + 4.0;
    let rows_y = text_y + 34.0;
    let mut rows = [[0.0; 4]; 3];
    let mut down = [[0.0; 4]; 3];
    let mut up = [[0.0; 4]; 3];
    for i in 0..3 {
        let ry = rows_y + i as f32 * 17.0;
        rows[i] = [rx, ry, rw, 15.0];
        down[i] = [rx + rw - 34.0, ry + 1.0, 15.0, 13.0];
        up[i] = [rx + rw - 16.0, ry + 1.0, 15.0, 13.0];
    }
    let by = rows_y + 3.0 * 17.0 + 4.0;
    let bw = ((rw - 16.0) / 5.0).floor();
    // As tall as what is in it, with the hint's line under it.
    let fit = (top + n * row_h).max(by + 15.0) + 20.0 - y;
    Menu {
        panel: [x, y, pw, ph.min(fit).floor()],
        list,
        banner,
        text_y,
        rows,
        down,
        up,
        key: [rx, by, bw, 15.0],
        films: [rx + bw + 4.0, by, bw, 15.0],
        reset: [rx + 2.0 * (bw + 4.0), by, bw, 15.0],
        quality: [rx + 3.0 * (bw + 4.0), by, bw, 15.0],
        terrain: [rx + 4.0 * (bw + 4.0), by, bw, 15.0],
    }
}

fn inside(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]
}

impl Menu {
    fn spot(&self, p: [f32; 2]) -> Option<Spot> {
        if let Some(i) = self.list.iter().position(|r| inside(*r, p)) {
            return Some(Spot::Skill(i as u8));
        }
        for (i, knob) in KNOBS.iter().enumerate() {
            if inside(self.down[i], p) {
                return Some(Spot::Down(*knob));
            }
            if inside(self.up[i], p) {
                return Some(Spot::Up(*knob));
            }
            if inside(self.rows[i], p) {
                return Some(Spot::Row(*knob));
            }
        }
        [(self.key, Spot::Key), (self.films, Spot::Films), (self.reset, Spot::Reset), (self.quality, Spot::Quality), (self.terrain, Spot::Terrain)].into_iter().find(|(r, _)| inside(*r, p)).map(|(_, s)| s)
    }
}

/// A tuning moved one notch (`dir` 1 up, -1 down) and kept.
fn nudge(game: &super::Game<'_>, skill: Skill, knob: Knob, dir: i32) {
    let mut t = tune(skill);
    match knob {
        Knob::Power if skill.power().min < skill.power().max => t.power = skill.power().step(t.power, dir),
        Knob::Power => return,
        Knob::Speed => t.speed = sk::SPEED.step(t.speed, dir),
        Knob::Damage => {
            let at = (sk::damage_step(t.damage) as i32 + dir).clamp(0, sk::DAMAGE_STEPS.len() as i32 - 1);
            t.damage = sk::DAMAGE_STEPS[at as usize];
        }
    }
    if t != tune(skill) {
        set_tune(skill, t);
        save(game);
        ui_sound("ws_bind", 1.0 + 0.04 * dir as f32);
    }
}

thread_local! {
    /// The menu's own clicks and chimes, played on the next tick (a screen's hooks only look).
    static UI_SOUNDS: std::cell::RefCell<Vec<(&'static str, f32)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The menu spot under the pointer last frame, for its hover tick.
    static HOVERED: std::cell::Cell<Option<Spot>> = const { std::cell::Cell::new(None) };
}

fn ui_sound(name: &'static str, pitch: f32) {
    UI_SOUNDS.with(|s| s.borrow_mut().push((name, pitch)));
}

/// The menu's queued chimes, in the head.
fn play_ui_sounds(game: &mut super::Game<'_>) {
    for (name, pitch) in UI_SOUNDS.with(|s| std::mem::take(&mut *s.borrow_mut())) {
        game.audio.play(&format!("shooting_star_demo:{name}"), None, 0.6, pitch);
    }
}

fn recharge(skill: Skill) -> (u32, u32) {
    let (left, total) = COOLDOWNS.with(|c| c.borrow()[skill.index() as usize]);
    (left, if total > 1 { total } else { skill.cooldown() as u32 })
}

fn banner_sprite(skill: Skill) -> &'static str {
    match skill {
        Skill::ShootingStar => "shooting_star_demo:gui/sprites/skill/stellar_remote/the_shooting_star",
        Skill::Gungnir => "shooting_star_demo:gui/sprites/skill/stellar_remote/gungnir",
        Skill::SevenStars => "shooting_star_demo:gui/sprites/skill/stellar_remote/seven_stars",
        Skill::Tesseract => "shooting_star_demo:gui/sprites/skill/stellar_remote/tesseract",
        Skill::Gravitia => "shooting_star_demo:gui/sprites/skill/stellar_remote/gravitia",
        // The port's own picture, made for it.
        Skill::EventHorizon => "shooting_star_demo:gui/sprites/skill/stellar_remote/event_horizon",
        Skill::Supernova => "shooting_star_demo:gui/sprites/skill/stellar_remote/supernova",
    }
}

fn argb_of(rgb: u32, alpha: u32) -> u32 {
    (alpha << 24) | (rgb & 0xFF_FFFF)
}

fn rgba_of(argb: u32) -> u32 {
    let [a, r, g, b] = argb.to_be_bytes();
    crate::hud::rgba(r, g, b, a)
}

/// A frame of one pixel round a rectangle.
fn outline(batch: &mut crate::hud::HudBatch, atlas: &crate::hud::HudAtlas, r: [f32; 4], argb: u32) {
    let c = rgba_of(argb);
    batch.fill(atlas, r[0], r[1], r[2], 1.0, c);
    batch.fill(atlas, r[0], r[1] + r[3] - 1.0, r[2], 1.0, c);
    batch.fill(atlas, r[0], r[1], 1.0, r[3], c);
    batch.fill(atlas, r[0] + r[2] - 1.0, r[1], 1.0, r[3], c);
}

fn button(batch: &mut crate::hud::HudBatch, atlas: &crate::hud::HudAtlas, r: [f32; 4], label: &str, accent: u32, hot: bool, off: bool) {
    let bg = if off { 0xC0181820 } else if hot { argb_of(accent, 0x60) } else { 0xC0202032 };
    batch.fill(atlas, r[0], r[1], r[2], r[3], rgba_of(bg));
    outline(batch, atlas, r, if off { 0xFF303040 } else if hot { argb_of(accent, 0xFF) } else { 0xFF4A4A66 });
    let w = crate::hud::HudBatch::text_width(atlas, label);
    let color = if off { 0xFF5A5A6A } else { 0xFFFFFFFF };
    batch.text(atlas, label, (r[0] + (r[2] - w) / 2.0).floor(), r[1] + ((r[3] - 8.0) / 2.0).floor(), color, false);
}

/// Words cut to lines no wider than `width` GUI pixels.
fn wrap_px(atlas: &crate::hud::HudAtlas, text: &str, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let tried = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        if !line.is_empty() && crate::hud::HudBatch::text_width(atlas, &tried) > width {
            out.push(std::mem::replace(&mut line, word.to_owned()));
        } else {
            line = tried;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn knob_text(skill: Skill, knob: Knob, t: &Tuning) -> (&'static str, String, u32) {
    match knob {
        Knob::Power => ("Power", skill.tune_line(t.power), 0xFFFFD470),
        Knob::Damage => ("Damage", if t.damage.is_infinite() { "True Erasure".to_owned() } else { format!("{:.0} ({:.1} hearts)", t.damage, t.damage / 2.0) }, 0xFFFF9A7A),
        Knob::Speed => ("Speed", format!("{:.2}x", t.speed), 0xFF9AD8FF),
    }
}

/// The `screen_render_post` hook: the spell menu.
pub fn menu_render(game: &super::Game<'_>, batch: &mut crate::hud::HudBatch, screen: &super::NativeScreenView<'_>) {
    if !menu_open(game) {
        return;
    }
    let atlas = &game.gui.atlas;
    let m = layout(batch.width, batch.height);
    let [x, y, w, h] = m.panel;
    let chosen = selected();
    let accent = chosen.color();
    let hot = m.spot(screen.mouse);
    if hot != HOVERED.with(|h| h.replace(hot)) && hot.is_some() {
        ui_sound("ws_hover", 1.0);
    }
    // The panel, edged in the chosen skill's colour.
    batch.fill(atlas, 0.0, 0.0, batch.width, batch.height, crate::hud::rgba(0, 0, 0, 110));
    batch.fill(atlas, x, y, w, h, crate::hud::rgba(10, 10, 18, 238));
    outline(batch, atlas, m.panel, argb_of(accent, 0x90));
    batch.fill(atlas, x, y, w, 2.0, rgba_of(argb_of(accent, 0xFF)));
    batch.text(atlas, "✦ STELLAR REMOTE", x + 8.0, y + 9.0, argb_of(accent, 0xFF), true);
    let version = format!("The Shooting Star {VERSION}");
    let vw = crate::hud::HudBatch::text_width(atlas, &version);
    batch.text(atlas, &version, x + w - vw - 8.0, y + 9.0, 0xFF6A6A80, false);

    // The skills, each a card: its picture, name, key and recharge.
    for skill in Skill::ALL {
        let i = skill.index();
        let r = m.list[i as usize];
        let on = skill == chosen;
        let over = hot == Some(Spot::Skill(i));
        let color = skill.color();
        batch.fill(atlas, r[0], r[1], r[2], r[3], rgba_of(if on { argb_of(color, 0x38) } else if over { 0x60282840 } else { 0x40181826 }));
        if on {
            outline(batch, atlas, r, argb_of(color, 0xFF));
            batch.fill(atlas, r[0], r[1], 2.0, r[3], rgba_of(argb_of(color, 0xFF)));
        }
        let th = r[3] - 4.0;
        let thumb = [r[0] + 4.0, r[1] + 2.0, (th * 2.0).floor(), th];
        if !game.native_sprite(batch, banner_sprite(skill), [0.0, 0.0, 256.0, 128.0], thumb) {
            batch.fill(atlas, thumb[0], thumb[1], thumb[2], thumb[3], rgba_of(argb_of(color, 0x60)));
        }
        let tx = thumb[0] + thumb[2] + 5.0;
        batch.text(atlas, skill.title(), tx, r[1] + 3.0, argb_of(color, 0xFF), true);
        let label = key_label(&skill_key(game, skill));
        let waiting = BINDING.with(std::cell::Cell::get) == Some(i);
        let key_line = if waiting { "press a key…".to_owned() } else if label == "No key" { "no key".to_owned() } else { format!("key {label}") };
        batch.text(atlas, &key_line, tx, r[1] + 13.0, if waiting { 0xFFFFE070 } else { 0xFF8A9AB8 }, false);
        // The recharge bar.
        let (left, total) = recharge(skill);
        let bar = [tx, r[1] + r[3] - 6.0, r[0] + r[2] - tx - 5.0, 2.0];
        batch.fill(atlas, bar[0], bar[1], bar[2], bar[3], crate::hud::rgba(40, 40, 56, 255));
        let ready = 1.0 - left as f32 / total.max(1) as f32;
        batch.fill(atlas, bar[0], bar[1], (bar[2] * ready).floor(), bar[3], rgba_of(if left > 0 { 0xFF8A8A9A } else { 0xFF7CFF8A }));
        if left > 0 {
            let s = format!("{}s", left.div_ceil(20));
            let sw = crate::hud::HudBatch::text_width(atlas, &s);
            batch.text(atlas, &s, r[0] + r[2] - sw - 5.0, r[1] + 13.0, 0xFF9A9A9A, false);
        }
    }

    // The chosen skill: its picture, its words and its tuning.
    let b = m.banner;
    if !game.native_sprite(batch, banner_sprite(chosen), [0.0, 0.0, 256.0, 128.0], b) {
        batch.fill(atlas, b[0], b[1], b[2], b[3], rgba_of(argb_of(accent, 0x40)));
    }
    outline(batch, atlas, b, argb_of(accent, 0xA0));
    batch.fill(atlas, b[0], b[1] + b[3] - 14.0, b[2], 14.0, crate::hud::rgba(0, 0, 0, 150));
    batch.text(atlas, chosen.title(), b[0] + 5.0, b[1] + b[3] - 11.0, 0xFFFFFFFF, true);
    let (left, _) = recharge(chosen);
    let state = if left > 0 { format!("recharging · {}s", left.div_ceil(20)) } else { "ready".to_owned() };
    let sw = crate::hud::HudBatch::text_width(atlas, &state);
    batch.text(atlas, &state, b[0] + b[2] - sw - 5.0, b[1] + b[3] - 11.0, if left > 0 { 0xFFB0B0B0 } else { 0xFF7CFF8A }, true);
    let mut ty = m.text_y;
    let (words, color) = if chosen.warning().is_empty() { (chosen.description(), 0xFFB8B8C8) } else { (chosen.warning(), 0xFFFF6A6A) };
    for line in wrap_px(atlas, words, b[2]).into_iter().take(3) {
        batch.text(atlas, &line, b[0], ty, color, false);
        ty += 10.0;
    }
    let t = tune(chosen);
    for (i, knob) in KNOBS.iter().enumerate() {
        let r = m.rows[i];
        if hot == Some(Spot::Row(*knob)) || hot == Some(Spot::Down(*knob)) || hot == Some(Spot::Up(*knob)) {
            batch.fill(atlas, r[0], r[1], r[2], r[3], crate::hud::rgba(255, 255, 255, 14));
        }
        let (name, value, c) = knob_text(chosen, *knob, &t);
        batch.text(atlas, name, r[0] + 3.0, r[1] + 4.0, c, false);
        let room = m.down[i][0] - (r[0] + 52.0) - 4.0;
        let mut shown = value.clone();
        while crate::hud::HudBatch::text_width(atlas, &shown) > room && shown.chars().count() > 4 {
            shown.pop();
        }
        if shown != value {
            shown.push('…');
        }
        batch.text(atlas, &shown, r[0] + 52.0, r[1] + 4.0, 0xFFE8E8F0, false);
        let fixed = *knob == Knob::Power && chosen.power().min >= chosen.power().max;
        button(batch, atlas, m.down[i], "-", accent, hot == Some(Spot::Down(*knob)), fixed);
        button(batch, atlas, m.up[i], "+", accent, hot == Some(Spot::Up(*knob)), fixed);
    }
    let key = key_label(&skill_key(game, chosen));
    let binding = BINDING.with(std::cell::Cell::get) == Some(chosen.index());
    button(batch, atlas, m.key, &if binding { "Press a key…".to_owned() } else { format!("Key: {key}") }, accent, hot == Some(Spot::Key) || binding, false);
    let films = CUTSCENES.with(std::cell::Cell::get);
    button(batch, atlas, m.films, if films { "Films: On" } else { "Films: Off" }, accent, hot == Some(Spot::Films), false);
    button(batch, atlas, m.reset, "Reset", accent, hot == Some(Spot::Reset), false);
    let land = TERRAIN.with(std::cell::Cell::get);
    button(batch, atlas, m.terrain, if land { "Land: On" } else { "Land: Off" }, accent, hot == Some(Spot::Terrain), false);
    let q = ["FX: Low", "FX: Med", "FX: High", "FX: Ultra"][usize::from(quality())];
    button(batch, atlas, m.quality, q, accent, hot == Some(Spot::Quality), false);

    // What the pointer is over, said at the bottom.
    let hint = if binding {
        "Press a key for this skill · Backspace: no key · Esc: cancel".to_owned()
    } else {
        match hot {
            Some(Spot::Skill(_)) => "Click: choose · Right-click: set its key · Wheel: next / previous".to_owned(),
            Some(Spot::Down(_) | Spot::Up(_) | Spot::Row(_)) => "Click - / + or turn the wheel over the row · Shift: five notches".to_owned(),
            Some(Spot::Key) => "Click, then press the key that picks this skill".to_owned(),
            Some(Spot::Films) => "The cutscenes in space; the cutscene key skips one".to_owned(),
            Some(Spot::Reset) => "Power, damage and speed back to the original's".to_owned(),
            Some(Spot::Quality) => "Effects quality: Low and Med for weaker GPUs, Ultra draws at full resolution".to_owned(),
            Some(Spot::Terrain) => "Off: spells change no blocks (effects and damage stay) · /tss undo restores the last spell's land".to_owned(),
            None => format!("↑↓ choose · ←→ power · {} closes", key_label(&mod_key(game, "skill_menu"))),
        }
    };
    let hw = crate::hud::HudBatch::text_width(atlas, &hint);
    batch.text(atlas, &hint, (x + (w - hw) / 2.0).floor(), y + h - 12.0, 0xFF7A7A90, false);
}

/// The `screen_mouse_button_pressed_pre` hook: a click on the spell menu.
pub fn menu_click(game: &super::Game<'_>, screen: &super::NativeScreenView<'_>, button: i32, canceled: &mut bool) {
    if !menu_open(game) {
        return;
    }
    *canceled = true;
    let m = layout(screen.rect[2], screen.rect[3]);
    let chosen = selected();
    let notches = if SHIFT.with(std::cell::Cell::get) { 5 } else { 1 };
    match m.spot(screen.mouse) {
        Some(Spot::Skill(i)) if button == 1 => {
            BINDING.with(|b| b.set(Some(i)));
            ui_sound("ws_info_open", 1.0);
        }
        // The click's hook only lends the game: the pick is made next tick.
        Some(Spot::Skill(i)) => PENDING.with(|p| p.set(Some(i))),
        Some(Spot::Down(k)) => (0..notches).for_each(|_| nudge(game, chosen, k, -1)),
        Some(Spot::Up(k)) => (0..notches).for_each(|_| nudge(game, chosen, k, 1)),
        Some(Spot::Row(k)) => (0..notches).for_each(|_| nudge(game, chosen, k, if button == 1 { -1 } else { 1 })),
        Some(Spot::Key) => {
            BINDING.with(|b| b.set(Some(chosen.index())));
            ui_sound("ws_info_open", 1.0);
        }
        Some(Spot::Films) => {
            CUTSCENES.with(|c| c.set(!c.get()));
            save(game);
            ui_sound("ws_bound", 1.0);
        }
        Some(Spot::Terrain) => {
            TERRAIN.with(|c| c.set(!c.get()));
            DIRTY.with(|d| d.set(true));
            save(game);
            ui_sound("ws_bound", 1.0);
        }
        Some(Spot::Reset) => {
            set_tune(chosen, Tuning::initial(chosen));
            save(game);
            ui_sound("ws_warp_out", 1.0);
        }
        Some(Spot::Quality) => {
            let step = if button == 1 { 3 } else { 1 };
            QUALITY.with(|q| q.set((q.get().min(3) + step) % 4));
            save(game);
            ui_sound("ws_bound", 1.0);
        }
        None => {}
    }
}

/// The `screen_mouse_scrolled_pre` hook: the wheel over a tuning moves it,
/// over the skills picks the next or the last.
pub fn menu_scroll(game: &super::Game<'_>, screen: &super::NativeScreenView<'_>, scroll: [f64; 2], canceled: &mut bool) {
    if !menu_open(game) || scroll[1] == 0.0 {
        return;
    }
    *canceled = true;
    let dir = if scroll[1] > 0.0 { 1 } else { -1 };
    let m = layout(screen.rect[2], screen.rect[3]);
    match m.spot(screen.mouse) {
        Some(Spot::Down(k) | Spot::Up(k) | Spot::Row(k)) => nudge(game, selected(), k, dir),
        Some(Spot::Skill(_)) | None => {
            let n = Skill::ALL.len() as i32;
            let next = (i32::from(selected().index()) - dir).rem_euclid(n);
            PENDING.with(|p| p.set(Some(next as u8)));
        }
        _ => {}
    }
}

/// Arrow keys while the menu is up: ↑↓ the skill, ←→ its power.
fn menu_key(game: &mut super::Game<'_>, key: &str) -> bool {
    let skill = selected();
    let n = Skill::ALL.len() as u8;
    match key {
        "key.keyboard.up" => select(game, Skill::ALL[usize::from((skill.index() + n - 1) % n)]),
        "key.keyboard.down" => select(game, skill.next()),
        "key.keyboard.left" => nudge(game, skill, Knob::Power, -1),
        "key.keyboard.right" => nudge(game, skill, Knob::Power, 1),
        _ => return false,
    }
    true
}

/// The `tooltip` hook: the remote's page, as the original's
/// `MagicItem.appendHoverText`.
pub fn tooltip(game: &super::Game<'_>, stack: &super::ItemStack, lines: &mut Vec<(crate::text::Text, u32)>) {
    if !is_remote(&stack.item) {
        return;
    }
    lines.push((
        crate::text::Text::literal("A weathered TV remote with a flip cover. Probably harmless... right?".to_owned()),
        0xFFAA00AA,
    ));
    lines.push((crate::text::Text::literal(String::new()), 0xFF555555));
    let chosen = selected();
    for skill in Skill::ALL {
        let label = key_label(&skill_key(game, skill));
        let key = if label == "No key" { "      ".to_owned() } else { format!("[{label}] ") };
        let mark = if skill == chosen { "▶ " } else { "" };
        lines.push((crate::text::Text::literal(format!("{mark}{key}The Shooting Star: {}", skill.title())), 0xFF00_0000 | skill.color()));
    }
    for chunk in wrap(chosen.description(), 48) {
        lines.push((crate::text::Text::literal(chunk), 0xFFB0B0B0));
    }
    for chunk in wrap(chosen.warning(), 48) {
        lines.push((crate::text::Text::literal(chunk), 0xFFFF7060));
    }
    lines.push((crate::text::Text::literal(String::new()), 0xFF555555));
    let menu_key = key_label(&mod_key(game, "skill_menu"));
    lines.push((crate::text::Text::literal(format!("Right-click: cast · {menu_key}: menu · Sneak + right-click: cycle")), 0xFF6E6E6E));
}

/// Break a long line on its spaces, so a tooltip keeps to the box.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

// ---- the HUD ------------------------------------------------------------------------------------------------------

/// The `hud` hook: the impact's flash, and the remote's indicator.
pub fn hud(game: &mut super::Game<'_>, batch: &mut crate::hud::HudBatch, corners: &mut crate::extras_hud::Corners) {
    if !game.native_window() {
        return;
    }
    let weight = film_weight();
    if weight > 0.001 {
        // The film's own corner: how to skip it (`CutsceneDirector.drawHud`).
        let a = ((weight * 2.0 - 1.0).clamp(0.0, 1.0) * 110.0) as u32;
        if a > 4 {
            let hint = format!("[{}] skip", key_label(&mod_key(game, "cutscenes")));
            let w = crate::hud::HudBatch::text_width(&game.gui.atlas, &hint);
            let (bw, bh) = (batch.width, batch.height);
            batch.text(&game.gui.atlas, &hint, bw - 18.0 - w, bh - 38.0, (a << 24) | 0x00FF_FFFF, false);
        }
        if weight > 0.35 {
            return;
        }
    }
    let skill = selected();
    let (left, _) = COOLDOWNS.with(|c| c.borrow()[skill.index() as usize]);
    let shown = SHOWN.with(std::cell::Cell::get).is_some_and(|when| when.elapsed() < std::time::Duration::from_secs(4));
    if left == 0 && !shown {
        return;
    }
    let ready = if left > 0 { format!("{}s left", left.div_ceil(20)) } else { "Ready".to_owned() };
    let selected_by = format!("Selected by {} · {}", key_label(&skill_key(game, skill)), skill.tune_line(tune(skill).power));
    let lines: Vec<(&str, u32)> = vec![
        (skill.title(), 0xFF00_0000 | skill.color()),
        (ready.as_str(), if left > 0 { 0xFF9A9A9A } else { 0xFF7CFF8A }),
        (selected_by.as_str(), 0xFF9AD8FF),
    ];
    crate::extras_hud::draw_lines(batch, &game.gui.atlas, corners, crate::extras::Corner::BottomRight, &lines);
}

// ---- the light in the world ---------------------------------------------------------------------------------------

fn argb(alpha: u32, red: u32, green: u32, blue: u32) -> u32 {
    (alpha.min(255) << 24) | (red.min(255) << 16) | (green.min(255) << 8) | blue.min(255)
}

fn ring(primitives: &mut crate::world_primitives::WorldPrimitives, at: [f64; 3], radius: f64, color: u32, width: f32) {
    let segments = 96;
    for step in 0..segments {
        let a0 = f64::from(step) / f64::from(segments) * std::f64::consts::TAU;
        let a1 = f64::from(step + 1) / f64::from(segments) * std::f64::consts::TAU;
        primitives.line(
            [at[0] + radius * a0.cos(), at[1], at[2] + radius * a0.sin()],
            [at[0] + radius * a1.cos(), at[1], at[2] + radius * a1.sin()],
            color,
            width,
        );
    }
}

/// SS-05: the square's grid, and the hypercube turning over it.
fn draw_grid(primitives: &mut crate::world_primitives::WorldPrimitives, spell: &ClientSpell, t: i32) {
    let start = &spell.start;
    let zone = sk::tess_zone(sk::tess_scale(start.power));
    let ox = f64::from(sk::tess_origin(start.target[0], zone));
    let oz = f64::from(sk::tess_origin(start.target[2], zone));
    let size = f64::from(zone);
    let y = start.target[1] + 0.3;
    let alpha = if t < 462 { 140 } else { ((1.0 - f64::from(t - 462) / 220.0).clamp(0.0, 1.0) * 200.0) as u32 };
    for step in 0..=8 {
        let o = size * f64::from(step) / 8.0;
        primitives.line([ox + o, y, oz], [ox + o, y, oz + size], argb(alpha, 127, 240, 255), 2.0);
        primitives.line([ox, y, oz + o], [ox + size, y, oz + o], argb(alpha, 127, 240, 255), 2.0);
    }
    if t >= 290 {
        let half = size / 8.0;
        let centre = [ox + size / 2.0, (start.target[1] + size * 0.6).min(300.0), oz + size / 2.0];
        let turn = f64::from(t) * 0.02;
        let (s, c) = turn.sin_cos();
        let corners: Vec<[f64; 3]> = (0..8)
            .map(|index| {
                let x = if index & 1 == 0 { -half } else { half };
                let yy = if index & 2 == 0 { -half } else { half };
                let z = if index & 4 == 0 { -half } else { half };
                [centre[0] + x * c - z * s, centre[1] + yy, centre[2] + x * s + z * c]
            })
            .collect();
        for (index, from) in corners.iter().enumerate() {
            for bit in [1usize, 2, 4] {
                let other = index ^ bit;
                if other > index {
                    primitives.line(*from, corners[other], argb(alpha.max(120), 200, 160, 255), 3.0);
                }
            }
        }
    }
}

/// SS-06: the ring engine closing round the world, then its blow.
fn draw_engine(primitives: &mut crate::world_primitives::WorldPrimitives, spell: &ClientSpell, cam: [f64; 3], t: i32) {
    let at = [cam[0], cam[1] + 60.0, cam[2]];
    let k = (f64::from(t) / 530.0).clamp(0.0, 1.0);
    let alpha = if t < 530 { (80.0 + 140.0 * k) as u32 } else { ((1.0 - f64::from(t - 530) / 120.0).clamp(0.0, 1.0) * 255.0) as u32 };
    let base = if t < 530 { 400.0 * (1.0 - 0.7 * k) } else { 120.0 + f64::from(t - 530) * 12.0 };
    for n in 0..3 {
        ring(primitives, [at[0], at[1] + f64::from(n) * 6.0, at[2]], base + f64::from(n) * 18.0, argb(alpha, 190, 120, 255), 6.0);
    }
    let _ = spell;
}

/// The world as the light needs it: the hour, how far the world is drawn,
/// the camera's projection, and whether this level is Imaginary Space.
fn light_context(game: &super::Game<'_>) -> super::shooting_star_visuals::Context {
    use super::shooting_star_gravitia as grav;
    let day_time = game.native_clock().map_or(18000, |c| c.day_time);
    let render_blocks = (game.settings.render_distance as f32).max(2.0) * 16.0;
    let dimension = game.native_player().map(|p| p.dimension.to_owned());
    let void = dimension.as_deref().and_then(|d| VOID.with(|v| v.borrow().get(d).copied()));
    let proj = game.native_camera().map(|c| {
        let (yaw, pitch) = (c.yaw.to_radians(), c.pitch.to_radians());
        let forward = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
        let right = [-yaw.cos(), 0.0, -yaw.sin()];
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        let p = c.projection;
        let tan_half = if p[5].abs() > 1e-6 { 1.0 / p[5] } else { (c.fov_degrees.to_radians() * 0.5).tan() };
        let aspect = if p[0].abs() > 1e-6 { p[5] / p[0] } else { 16.0 / 9.0 };
        grav::glam_like::Proj { forward, right, up, tan_half, aspect }
    });
    super::shooting_star_visuals::Context {
        world: grav::World { day_time, render_blocks, banished: void.is_some() },
        proj,
        void: void.map(|seed| {
            let a = f64::from(seed % 3600) / 10.0;
            let a = a.to_radians();
            (-a.sin(), a.cos())
        }),
    }
}

/// The spell's clock this frame, as the original's `SpellFx.time(partial)`.
fn spell_time(spell: &ClientSpell, partial: f32) -> f32 {
    let speed = spell.start.speed;
    if spell.age == 0 { partial * speed } else { (spell.clock - f64::from(speed) + 1.0 + f64::from(partial * speed)) as f32 }
}

/// The `world_primitives` hook, once a frame: the original's passes for every
/// spell playing go to the render pass (`shooting_star_render`), on the
/// spells' own clocks.
pub fn frame_tick(game: &super::Game<'_>, partial: f32, camera: [f64; 3], primitives: &mut crate::world_primitives::WorldPrimitives) {
    let (min_y, height) = game.native_world_height();
    let views: Vec<super::shooting_star_visuals::Spell> = SPELLS.with(|s| {
        s.borrow()
            .iter()
            .map(|spell| super::shooting_star_visuals::Spell {
                start: spell.start,
                t: spell_time(spell, partial),
                boomed: spell.boomed.map(|b| b as f32),
                film: {
                    let ct = spell_time(spell, partial);
                    film_weight_at(spell, ct).map(|weight| super::shooting_star_visuals::Film { ct, weight })
                },
            })
            .collect()
    });
    let time = (TICKS.with(std::cell::Cell::get) as f32 + partial) / 20.0;
    let context = light_context(game);
    let mut frame = super::shooting_star_visuals::frame_in(&views, camera, min_y, min_y + height - 1, time, &context);
    frame.quality = quality();
    if !assets_ok() {
        // Until the first run has the original's shaders, nothing to draw them with.
        frame.passes.clear();
    }
    let drew = !frame.passes.is_empty();
    horizon_client_render::gfx::native_render::shooting_star_fxframe::publish(frame);
    // Until the original's shaders are in, the Tesseract's grid and Gravitia's
    // rings are drawn as lines, that a cast is never invisible.
    if !assets_ok() {
        SPELLS.with(|s| {
            for spell in s.borrow().iter() {
                match spell.start.skill {
                    Skill::Tesseract if (9..682).contains(&spell.age) => draw_grid(primitives, spell, spell.age),
                    Skill::Gravitia if (9..650).contains(&spell.age) => draw_engine(primitives, spell, camera, spell.age),
                    _ => {}
                }
            }
        });
    }
    if drew {
        DREW.with(|once| {
            if !once.replace(true) {
                log::info!("[{ID}] a spell's passes reach the renderer");
            }
        });
    }
}

// ---- /tss -------------------------------------------------------------------------------------------------------

/// `/tss`: the mod's own commands. `Err` when the line is not ours.
pub fn perform(game: &mut super::Game<'_>, command: &str) -> Result<String, ()> {
    let mut words = command.split_whitespace();
    if !words.next().is_some_and(|word| word.eq_ignore_ascii_case("tss")) {
        return Err(());
    }
    let usage = Skill::IDS.join("|");
    let answer = match words.next() {
        Some("info") => format!("The Shooting Star {VERSION} (Horizon port): {} skills", Skill::ALL.len()),
        Some("cast") => match words.next().and_then(Skill::by_id) {
            Some(skill) => match game.native_acting_player() {
                Some(player) => {
                    super::shooting_star_spells::cast(game, &player, skill, Tuning::initial(skill));
                    format!("Casting {}", skill.title())
                }
                None => "Only a player can cast".to_owned(),
            },
            None => format!("Usage: /tss cast <{usage}>"),
        },
        Some("select") => match words.next().and_then(Skill::by_id) {
            Some(skill) => {
                if let Some(player) = game.native_acting_player() {
                    super::shooting_star_spells::on_client_message(player.uuid, CH_CMD, &[sk::CMD_SELECT, skill.index()]);
                    let _ = game.native_net_send_to_player(player.uuid, CHANNEL, &[sk::FX_SELECT, skill.index()]);
                }
                format!("Selected {}", skill.title())
            }
            None => format!("Usage: /tss select <{usage}>"),
        },
        Some("trailer") => match (words.next(), game.native_acting_player()) {
            (Some("stop"), _) => {
                super::shooting_star_spells::stop_trailer(game);
                "The trailer is stopped".to_owned()
            }
            (_, Some(player)) => {
                super::shooting_star_spells::start_trailer(game, &player);
                "The trailer starts: every skill in turn, about four minutes. Leave the mouse alone.".to_owned()
            }
            (_, None) => "Only a player can film the trailer".to_owned(),
        },
        Some("next") => match game.native_acting_player() {
            Some(player) => {
                let next = super::shooting_star_spells::cycle(player.uuid);
                let _ = game.native_net_send_to_player(player.uuid, CHANNEL, &[sk::FX_SELECT, next.index()]);
                format!("Selected {}", next.title())
            }
            None => "Only a player has a remote".to_owned(),
        },
        Some("undo") => match game.native_acting_player() {
            Some(player) => super::shooting_star_spells::undo(game, player.uuid),
            None => "Only a player can undo".to_owned(),
        },
        _ => "Usage: /tss <info|cast|select|next|undo>".to_owned(),
    };
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remote_is_recognised() {
        assert!(is_remote(REMOTE_ID));
        assert!(!is_remote("minecraft:stick"));
    }

    #[test]
    fn a_far_spell_is_heard_from_its_edge() {
        let e = ear([300.0, 70.0, 0.0], [0.0, 64.0, 0.0], 200.0);
        assert!((e[0] - 200.0).abs() < 1e-9 && e[1] == 70.0);
        assert_eq!(ear([10.0, 70.0, 0.0], [0.0, 64.0, 0.0], 200.0), [10.0, 70.0, 0.0]);
    }

    #[test]
    fn the_original_keys() {
        assert_eq!(Skill::ShootingStar.default_key(), "key.keyboard.k");
        assert!(KEYS.iter().any(|(n, _, d)| *n == "cutscenes" && *d == "key.keyboard.j"));
        assert_eq!(key_label("key.keyboard.k"), "K");
    }
}
