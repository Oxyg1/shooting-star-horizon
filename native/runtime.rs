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
pub const VERSION: &str = "1.5.2-hz6";

/// The Stellar Remote, declared by the companion content mod of the same
/// namespace (`shooting_star_demo`, data/shooting_star_demo/horizon/item).
pub const REMOTE_ID: &str = "shooting_star_demo:stellar_remote";

/// The command usages the mod adds to the server's tree.
pub const COMMANDS: &[&str] = &["tss info", "tss cast <skill:word>", "tss select <skill:word>", "tss next"];

/// The server to the window, and the window to the server.
pub const CHANNEL: &str = "shooting_star:fx";
pub const CH_CMD: &str = "shooting_star:cmd";

/// The mod's keys: name, label and default, as the mods page lists them. The
/// first seven are the original's (H menu, J cutscenes, K I U Y skills; O
/// for Gravitia); the tuning keys stand in for its menu's sliders.
pub const KEYS: [(&str, &str, &str); 11] = [
    ("skill_menu", "Open skill menu", "key.keyboard.h"),
    ("cutscenes", "Skip / toggle spell cutscenes", "key.keyboard.j"),
    ("the_shooting_star", "The Shooting Star: SS-01 Railgun", "key.keyboard.k"),
    ("gungnir", "The Shooting Star: SS-03 Gungnir", "key.keyboard.i"),
    ("seven_stars", "The Shooting Star: SS-04 Seven Stars", "key.keyboard.u"),
    ("tesseract", "The Shooting Star: SS-05 Tesseract", "key.keyboard.y"),
    ("gravitia", "The Shooting Star: SS-06 Gravitia", "key.keyboard.o"),
    ("cycle", "Next skill", "key.keyboard.g"),
    ("power", "Selected skill: power (Shift: down)", "key.keyboard.n"),
    ("speed", "Selected skill: speed (Shift: down)", "key.keyboard.m"),
    ("damage", "Selected skill: damage (Shift: down)", "key.keyboard.z"),
];

// ---- the window's remote ------------------------------------------------------------------------------------------

thread_local! {
    static SELECTED: std::cell::Cell<Skill> = const { std::cell::Cell::new(Skill::ShootingStar) };
    static TUNES: std::cell::RefCell<[Tuning; 5]> = std::cell::RefCell::new([
        Tuning::initial(Skill::ShootingStar),
        Tuning::initial(Skill::Gungnir),
        Tuning::initial(Skill::SevenStars),
        Tuning::initial(Skill::Tesseract),
        Tuning::initial(Skill::Gravitia),
    ]);
    static CUTSCENES: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static LOADED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The selection and tuning wait to reach the server.
    static DIRTY: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Each skill's recharge as the server last told it: ticks left, ticks in all.
    static COOLDOWNS: std::cell::RefCell<[(u32, u32); 5]> = const { std::cell::RefCell::new([(0, 1); 5]) };
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
    out.push_str(&format!("selected={}\n", selected().id()));
    for skill in Skill::ALL {
        let t = tune(skill);
        let damage = if t.damage.is_infinite() { "infinite".to_owned() } else { t.damage.to_string() };
        out.push_str(&format!("{0}.power={1}\n{0}.damage={2}\n{0}.speed={3}\n", skill.id(), t.power, damage, t.speed));
    }
    if let Err(error) = std::fs::write(&path, out) {
        log::warn!("[{ID}] the remote's settings could not be saved: {error}");
    }
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
        let said = if cleared { "No key · right-click to set one".to_owned() } else { format!("Selected by {}", key_label(key)) };
        game.native_client_action_bar(crate::text::Text::literal(said), true);
        return;
    }
    let menu = menu_open(game);
    if !menu && !game.native_playing() {
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
        sk::FX_HELLO => DIRTY.with(|d| d.set(true)),
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
    load_assets(game);
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
        if a.is_ok() && b.is_ok() {
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
        while spell.age <= until && spell.age <= spell.start.skill.duration() + 40 {
            let t = spell.age;
            cue(game, spell, t, now);
            spell.age += 1;
        }
        spell.clock += f64::from(spell.start.speed);
        shockwave(game, spell);
    }
    spells.retain(|s| s.age <= s.start.skill.duration() + 40);
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

/// How much a film holds the camera this frame.
pub fn film_weight() -> f32 {
    LENS.with(std::cell::Cell::get).0
}

/// Picks the film to play, each tick: a new spell of this player's starts
/// its own, the last one's ends with it.
fn direct(spells: &[ClientSpell], heights: (i32, i32)) {
    let newest = spells.iter().rev().find(|s| s.start.mine && s.start.skill != Skill::Gravitia);
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

/// The original's `sound(...)` out in the world: its volume times 16, so it
/// carries 256 blocks (16 times the event's own reach), loud up to the ear.
fn play(game: &mut super::Game<'_>, name: &str, at: [f64; 3], volume: f32, pitch: f32) {
    game.native_play_local_sound(&format!("shooting_star_demo:{name}"), at, volume * 16.0, pitch, 0);
}

/// The original's `cue(...)` (`ScoreCue`): volume one, so it carries as far as
/// its own `sounds.json` entry says (256 blocks for the railgun's).
fn cue_at(game: &mut super::Game<'_>, name: &str, at: [f64; 3]) {
    game.native_play_local_sound(&format!("shooting_star_demo:{name}"), at, 1.0, 1.0, 0);
}

/// The original's `ears(...)` and film sounds: heard as if inside the head,
/// at the listener whatever the distance.
fn ears(game: &mut super::Game<'_>, name: &str, volume: f32) {
    let Some(cam) = camera(game) else { return };
    game.native_play_local_sound(&format!("shooting_star_demo:{name}"), cam, volume.min(1.0), 1.0, 0);
}

/// `sound(..., film)`: in the head during the film, out in the world else.
fn sound(game: &mut super::Game<'_>, film: bool, name: &str, at: [f64; 3], volume: f32, pitch: f32) {
    if film { ears(game, name, volume) } else { play(game, name, at, volume, pitch) }
}

fn jolt(at: [f64; 3], strength: f32, reach: f64, now: u64) {
    SHAKES.with(|s| s.borrow_mut().push(Jolt { at, strength, reach, born: now }));
}

/// One tick of a spell's sound and jolts: the original's `StarFx`,
/// `GungnirFx`, `SevenStarsFx`, the Tesseract's score and Gravitia's.
fn cue(game: &mut super::Game<'_>, spell: &mut ClientSpell, t: i32, now: u64) {
    let Some(cam) = camera(game) else { return };
    let start = spell.start;
    let film = film_on(spell);
    let zone = zone_of(&start);
    let edge = if film { cam } else { ear(cam, start.target, zone) };
    let caster = start.origin;
    let target = start.target;
    let far = (cam[0] - target[0]).hypot(cam[2] - target[2]);
    match start.skill {
        Skill::ShootingStar => {
            match t {
                0 => cue_at(game, "star_rg_arm", caster),
                7 => cue_at(game, "star_rg_press", caster),
                9 if film => ears(game, "star_lock", 1.0),
                9 => cue_at(game, "star_lock", edge),
                29 if !film => play(game, "star_countdown", edge, 1.0, 1.0),
                44 if film => ears(game, "star_ascent", 1.0),
                238 if film => ears(game, "star_gun", 1.0),
                314 if film => ears(game, "star_fire", 1.0),
                352 => {
                    if film { ears(game, "star_impact", 1.0) } else { play(game, "star_impact", edge, 1.0, 1.0) }
                    cue_at(game, "star_beam", edge);
                    jolt(edge, 1.0, 700.0, now);
                }
                462 => cue_at(game, "star_rg_collapse", edge),
                490 => cue_at(game, "star_rg_after", edge),
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
            if t == 0 {
                ears(game, "grav_rework", 1.0);
            }
            if t == 9 {
                play(game, "grav_mark", caster, 1.0, 1.0);
            }
            if t == 530 {
                jolt(cam, 3.0, 1.0e9, now);
            }
            if t > 530 && t < 620 && t % 6 == 0 {
                jolt(cam, 0.6, 1.0e9, now);
            }
        }
    }
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

fn menu_panel(width: f32, height: f32) -> [f32; 4] {
    let w = 300.0_f32.min(width - 16.0);
    let h = 236.0_f32.min(height - 16.0);
    [((width - w) / 2.0).floor(), ((height - h) / 2.0).floor(), w, h]
}

fn menu_row(panel: [f32; 4], index: u8) -> [f32; 4] {
    let row_h = 22.0;
    let top = panel[1] + 36.0;
    [panel[0] + 10.0, top + f32::from(index) * row_h, panel[2] - 20.0, row_h - 3.0]
}

fn menu_hover(panel: [f32; 4], mouse: [f32; 2]) -> Option<u8> {
    Skill::ALL.iter().map(|skill| skill.index()).find(|index| {
        let [x, y, w, h] = menu_row(panel, *index);
        mouse[0] >= x && mouse[0] <= x + w && mouse[1] >= y && mouse[1] <= y + h
    })
}

fn recharge_text(skill: Skill) -> (String, bool) {
    let (left, total) = COOLDOWNS.with(|c| c.borrow()[skill.index() as usize]);
    if left > 0 {
        (format!("{}s left", left.div_ceil(20)), true)
    } else {
        let total = if total > 1 { total } else { skill.cooldown() as u32 };
        (format!("Ready · Recharge {}s", total.div_ceil(20)), false)
    }
}

/// The `screen_render_post` hook: the spell menu.
pub fn menu_render(game: &super::Game<'_>, batch: &mut crate::hud::HudBatch, screen: &super::NativeScreenView<'_>) {
    if !menu_open(game) {
        return;
    }
    let atlas = &game.gui.atlas;
    let panel = menu_panel(batch.width, batch.height);
    let [x, y, w, h] = panel;
    batch.fill(atlas, x, y, w, h, crate::hud::rgba(8, 8, 16, 235));
    batch.fill(atlas, x, y, w, 2.0, crate::hud::rgba(255, 50, 72, 255));
    batch.fill(atlas, x, y + h - 2.0, w, 2.0, crate::hud::rgba(255, 50, 72, 255));
    batch.text(atlas, "The Shooting Star", x + 10.0, y + 8.0, 0xFFFF3248, true);
    batch.text(atlas, &format!("Stellar Remote · {} skills · {VERSION}", Skill::ALL.len()), x + 10.0, y + 20.0, 0xFF9A9A9A, false);
    let chosen = selected();
    let hovered = menu_hover(panel, screen.mouse);
    for skill in Skill::ALL {
        let index = skill.index();
        let [rx, ry, rw, _] = menu_row(panel, index);
        if skill == chosen {
            batch.fill(atlas, rx, ry, rw, 19.0, crate::hud::rgba(70, 20, 30, 220));
        } else if hovered == Some(index) {
            batch.fill(atlas, rx, ry, rw, 19.0, crate::hud::rgba(40, 40, 60, 200));
        }
        let color = 0xFF00_0000 | skill.color();
        batch.text(atlas, skill.title(), rx + 6.0, ry + 3.0, color, true);
        let (state, waiting) = recharge_text(skill);
        let width = crate::hud::HudBatch::text_width(atlas, &state);
        batch.text(atlas, &state, rx + rw - width - 6.0, ry + 3.0, if waiting { 0xFF9A9A9A } else { 0xFF7CFF8A }, false);
        let label = key_label(&skill_key(game, skill));
        let key_text = if label == "No key" { "No key · right-click to set one".to_owned() } else { format!("Selected by {label}") };
        batch.text(atlas, &key_text, rx + 6.0, ry + 11.0, if label == "No key" { 0xFF8A8A8A } else { 0xFF9AD8FF }, false);
    }
    // The tuning page of the selected skill.
    let t = tune(chosen);
    let mut line_y = menu_row(panel, 5)[1] + 4.0;
    let lines = [
        (format!("Power [N]: {}", chosen.tune_line(t.power)), 0xFFFFD470),
        (format!("Damage [Z]: {}", damage_line(t.damage)), 0xFFFF9A7A),
        (format!("Speed [M]: {:.2}x", t.speed), 0xFF9AD8FF),
        (
            format!("Cutscenes [{}]: {}", key_label(&mod_key(game, "cutscenes")), if CUTSCENES.with(std::cell::Cell::get) { "On" } else { "Off" }),
            0xFFB0B0B0,
        ),
    ];
    for (text, color) in lines.iter() {
        batch.text(atlas, text, x + 10.0, line_y, *color, false);
        line_y += 10.0;
    }
    if !chosen.warning().is_empty() {
        for chunk in wrap(chosen.warning(), 58) {
            batch.text(atlas, &chunk, x + 10.0, line_y, 0xFFFF6060, false);
            line_y += 10.0;
        }
    }
    let hint = match BINDING.with(std::cell::Cell::get) {
        Some(index) => {
            let name = Skill::by_index(index).map_or("the spell", |s| s.title());
            format!("Press a key for {name} · Backspace clears it · Esc cancels")
        }
        None => "Click a skill to select it · Right-click to set its key · Shift+N/M/Z: down".to_owned(),
    };
    batch.text(atlas, &hint, x + 10.0, y + h - 12.0, 0xFF7A7A7A, false);
}

/// The `screen_mouse_button_pressed_pre` hook: a click on the spell menu.
pub fn menu_click(game: &super::Game<'_>, screen: &super::NativeScreenView<'_>, button: i32, canceled: &mut bool) {
    if !menu_open(game) {
        return;
    }
    *canceled = true;
    let panel = menu_panel(screen.rect[2], screen.rect[3]);
    let Some(index) = menu_hover(panel, screen.mouse) else {
        return;
    };
    if button == 1 {
        BINDING.with(|b| b.set(Some(index)));
        return;
    }
    if button == 0 {
        // The click's hook only lends the game: the pick is made next tick.
        PENDING.with(|p| p.set(Some(index)));
    }
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
    let mut frame = super::shooting_star_visuals::frame(&views, camera, min_y, min_y + height - 1, time);
    if !assets_ok() {
        // Until the first run has the original's shaders, nothing to draw them with.
        frame.passes.clear();
    }
    let drew = !frame.passes.is_empty();
    horizon_client_render::gfx::native_render::shooting_star_fxframe::publish(frame);
    // Tesseract and Gravitia keep their marks until their passes are ported.
    SPELLS.with(|s| {
        for spell in s.borrow().iter() {
            match spell.start.skill {
                Skill::Tesseract if (9..682).contains(&spell.age) => draw_grid(primitives, spell, spell.age),
                Skill::Gravitia if (9..650).contains(&spell.age) => draw_engine(primitives, spell, camera, spell.age),
                _ => {}
            }
        }
    });
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
        Some("next") => match game.native_acting_player() {
            Some(player) => {
                let next = super::shooting_star_spells::cycle(player.uuid);
                let _ = game.native_net_send_to_player(player.uuid, CHANNEL, &[sk::FX_SELECT, next.index()]);
                format!("Selected {}", next.title())
            }
            None => "Only a player has a remote".to_owned(),
        },
        _ => "Usage: /tss <info|cast|select|next>".to_owned(),
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
