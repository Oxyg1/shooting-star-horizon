// The first run: the original's files, fetched and made into the content mod.
//
// The repository carries none of the original's files. The first time the
// game starts with this mod and finds its content missing, it downloads the
// original's jars from CurseForge (checked by SHA-256, kept for later runs),
// builds the content mod from the files `files.tsv` names into
// `horizon-mods/shooting_star_demo/`, turns it on, and records the films'
// tables by running the original's own camera classes with Java 25 (the
// one Minecraft 26.3 and Horizon Launcher download). One restart later the
// Stellar Remote, its blocks and sounds are in the game; the light works at once.
//
// All of it on a thread of its own: the game never waits.
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MOD_ID: &str = "shooting_star_demo";

/// The original's jars on CurseForge (project 1719195): 1.5.2's for its
/// files, 1.4.4's for the shaders 1.5.2 takes from its library.
struct Jar {
    version: &'static str,
    url: &'static str,
    sha256: &'static str,
    file: &'static str,
    inner: &'static str,
}

const JARS: [Jar; 2] = [
    Jar {
        version: "1.5.2",
        url: "https://www.curseforge.com/api/v1/mods/1719195/files/9438887/download",
        sha256: "90d3c145ea00a3a5c4119b51f42b06cd0569c83733d2f3a5398579e77f4d703a",
        file: "the-shooting-star-demo-1.5.2-fabric.jar",
        inner: "META-INF/jars/the-shooting-star-demo-1.5.2-demo+mc26.3.jar",
    },
    Jar {
        version: "1.4.4",
        url: "https://www.curseforge.com/api/v1/mods/1719195/files/9231726/download",
        sha256: "1c361965467e66c0050609f5f9ce53ec4a70677ec74dc5f119da79bdc46efa1a",
        file: "the-shooting-star-demo-1.4.4-fabric.jar",
        inner: "META-INF/jars/the-shooting-star-demo-1.4.4-demo+mc26.3.jar",
    },
];

macro_rules! data {
    ($path:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../native-data/shooting_star/", $path))
    };
}

/// What goes where: `<path in the content mod> TAB <jar> TAB <path in it>`.
const FILES: &[u8] = data!("files.tsv");

/// The port's own content files (the blocks and the item as Horizon declares them).
const OWN: &[(&str, &[u8])] = &[
    ("horizon-mod.toml", data!("content/horizon-mod.toml")),
    ("assets/shooting_star_demo/items/stellar_remote.json", data!("content/assets/shooting_star_demo/items/stellar_remote.json")),
    ("data/shooting_star_demo/horizon/block/gravitia_space_block.json", data!("content/data/shooting_star_demo/horizon/block/gravitia_space_block.json")),
    ("data/shooting_star_demo/horizon/block/gungnir_hull.json", data!("content/data/shooting_star_demo/horizon/block/gungnir_hull.json")),
    ("data/shooting_star_demo/horizon/block/gungnir_seam.json", data!("content/data/shooting_star_demo/horizon/block/gungnir_seam.json")),
    ("data/shooting_star_demo/horizon/block/lattice_edge.json", data!("content/data/shooting_star_demo/horizon/block/lattice_edge.json")),
    ("data/shooting_star_demo/horizon/block/star_core.json", data!("content/data/shooting_star_demo/horizon/block/star_core.json")),
    ("data/shooting_star_demo/horizon/block/star_trace.json", data!("content/data/shooting_star_demo/horizon/block/star_trace.json")),
    ("data/shooting_star_demo/horizon/item/stellar_remote.json", data!("content/data/shooting_star_demo/horizon/item/stellar_remote.json")),
    // SS-H1's card: the port's own picture.
    (
        "assets/shooting_star_demo/textures/gui/sprites/skill/stellar_remote/event_horizon.png",
        data!("content/assets/shooting_star_demo/textures/gui/sprites/skill/stellar_remote/event_horizon.png"),
    ),
];

/// The films' recorder (`tools/films`), compiled.
const TOOL: &[(&str, &[u8])] = &[
    ("Films.class", data!("films-tool/Films.class")),
    ("Films$Source.class", data!("films-tool/Films$Source.class")),
    ("cyou/rimuru/rimulib/client/cinematic/CutsceneDirector.class", data!("films-tool/cyou/rimuru/rimulib/client/cinematic/CutsceneDirector.class")),
];

/// Where the first run is.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Working(String),
    /// Done: whether the films' tables were made too.
    Done { films: bool },
    Failed(String),
}

static STATUS: Mutex<Status> = Mutex::new(Status::Idle);

fn set(status: Status) {
    match STATUS.lock() {
        Ok(mut s) => *s = status,
        Err(poisoned) => *poisoned.into_inner() = status,
    }
}

pub fn status() -> Status {
    match STATUS.lock() {
        Ok(s) => s.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// The content mod's folder in the game folder.
pub fn content_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("horizon-mods").join(MOD_ID)
}

/// The content mod is there with the original's files.
pub fn ready(game_dir: &Path) -> bool {
    let data = content_dir(game_dir).join("data").join(MOD_ID);
    // hz9 added Gravitia's machine and hz10 SS-H1's card: an older content
    // mod is built again (from the jars already kept, nothing is downloaded).
    let card = content_dir(game_dir).join("assets/shooting_star_demo/textures/gui/sprites/skill/stellar_remote/event_horizon.png");
    data.join("fx").join("composite.fsh").is_file() && data.join("models").join("gravitia").join("machine.bin").is_file() && card.is_file()
}

/// Its films' tables are there.
pub fn films_ready(game_dir: &Path) -> bool {
    content_dir(game_dir).join("data").join(MOD_ID).join("films").join("tess_sky.bin").is_file()
}

/// `mod.shooting_star_demo:true` in `horizon-mods.txt`, once.
pub fn turn_on(game_dir: &Path) -> std::io::Result<bool> {
    let list = game_dir.join("horizon-mods.txt");
    let text = std::fs::read_to_string(&list).unwrap_or_default();
    let line = format!("mod.{MOD_ID}:true");
    if text.lines().any(|l| l.trim() == line) {
        return Ok(false);
    }
    let mut kept: Vec<&str> = text.lines().filter(|l| !l.starts_with(&format!("mod.{MOD_ID}:"))).collect();
    kept.push(&line);
    std::fs::write(&list, kept.join("\n") + "\n")?;
    Ok(true)
}

/// Starts the first run on a thread of its own, once.
pub fn start(game_dir: PathBuf, cache: PathBuf, films_only: bool) {
    {
        let mut s = match STATUS.lock() {
            Ok(s) => s,
            Err(poisoned) => poisoned.into_inner(),
        };
        if matches!(*s, Status::Working(_)) {
            return;
        }
        *s = Status::Working("Preparing the original's files".to_owned());
    }
    let spawned = std::thread::Builder::new().name("shooting star setup".into()).spawn(move || {
        let result = if films_only { films(&game_dir, &cache) } else { run(&game_dir, &cache) };
        // The original's jars stay (checked by their SHA-256 each time), so a
        // later rebuild of the content mod never downloads them again; only
        // the films' working copies go.
        for j in &JARS {
            let _ = std::fs::remove_file(cache.join(format!("mc26.3-{}.jar", j.version)));
        }
        let _ = std::fs::remove_dir_all(cache.join("films-tool"));
        match result {
            Ok(films) => set(Status::Done { films }),
            Err(error) => {
                log::warn!("[shooting_star] the first run failed: {error}");
                set(Status::Failed(error));
            }
        }
    });
    if let Err(error) = spawned {
        set(Status::Failed(error.to_string()));
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A jar of the original, from the cache or CurseForge.
fn jar(jar: &Jar, cache: &Path) -> Result<Vec<u8>, String> {
    let path = cache.join(jar.file);
    if let Ok(bytes) = std::fs::read(&path)
        && sha256(&bytes) == jar.sha256
    {
        return Ok(bytes);
    }
    // The one-line installers keep their own copy: no need to fetch it twice.
    for dir in installer_caches() {
        if let Ok(bytes) = std::fs::read(dir.join(jar.file))
            && sha256(&bytes) == jar.sha256
        {
            let _ = std::fs::create_dir_all(cache);
            let _ = std::fs::write(&path, &bytes);
            return Ok(bytes);
        }
    }
    set(Status::Working(format!("Downloading the original ({}) from CurseForge", jar.version)));
    // A mirror of the same files, when CurseForge can't be reached (checked by the same SHA-256).
    let url = match std::env::var("SHOOTING_STAR_MIRROR") {
        Ok(mirror) if !mirror.is_empty() => format!("{}/{}", mirror.trim_end_matches('/'), jar.file),
        _ => jar.url.to_owned(),
    };
    let mut agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(20)).timeout_read(std::time::Duration::from_secs(60));
    if let Some(proxy) = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"].iter().find_map(|v| std::env::var(v).ok().filter(|p| !p.is_empty()))
        && let Ok(proxy) = ureq::Proxy::new(proxy)
    {
        agent = agent.proxy(proxy);
    }
    let response = agent.build().get(&url).call().map_err(|e| format!("CurseForge could not be reached: {e}"))?;
    let total = response.header("Content-Length").and_then(|v| v.trim().parse::<u64>().ok());
    let mut reader = response.into_reader().take(256 << 20);
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; 1 << 16];
    let mut shown = 0;
    loop {
        let n = reader.read(&mut chunk).map_err(|e| format!("the download broke off: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        let mib = bytes.len() >> 20;
        if mib >= shown + 4 {
            shown = mib;
            let of = total.map(|t| format!(" of {}", t >> 20)).unwrap_or_default();
            set(Status::Working(format!("Downloading the original ({}): {mib}{of} MB", jar.version)));
        }
    }
    if sha256(&bytes) != jar.sha256 {
        return Err(format!("{}: not the file this port was made for (checksum)", jar.file));
    }
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let _ = std::fs::write(&path, &bytes);
    Ok(bytes)
}

/// Where `install.ps1` and `install.sh` keep the original's jars.
fn installer_caches() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        out.push(PathBuf::from(local).join("shooting-star-horizon"));
    }
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(xdg).join("shooting-star-horizon"));
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        out.push(PathBuf::from(home).join(".cache").join("shooting-star-horizon"));
    }
    out
}

/// The mc26.3 jar inside a jar of the original.
fn inner(outer: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(outer)).map_err(|e| e.to_string())?;
    let mut entry = zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    let mut out = Vec::new();
    entry.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::File::create(path).and_then(|mut f| f.write_all(bytes)).map_err(|e| format!("{}: {e}", path.display()))
}

/// The jars' inner jars, by version.
fn inner_jars(cache: &Path) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    let mut out = Vec::new();
    for j in &JARS {
        let outer = jar(j, cache)?;
        out.push((j.version, inner(&outer, j.inner)?));
    }
    Ok(out)
}

/// The whole first run: the content mod built, turned on, and its films.
fn run(game_dir: &Path, cache: &Path) -> Result<bool, String> {
    let jars = inner_jars(cache)?;
    set(Status::Working("Building the content mod".to_owned()));
    let target = content_dir(game_dir);
    let fresh = target.with_file_name(format!(".{MOD_ID}.new"));
    let _ = std::fs::remove_dir_all(&fresh);
    for (path, bytes) in OWN {
        write(&fresh.join(path), bytes)?;
    }
    let mut zips = Vec::new();
    for (version, bytes) in &jars {
        zips.push((*version, zip::ZipArchive::new(Cursor::new(bytes.as_slice())).map_err(|e| e.to_string())?));
    }
    let manifest = std::str::from_utf8(FILES).map_err(|e| e.to_string())?;
    let mut count = 0;
    for line in manifest.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('\t');
        let (Some(dest), Some(version), Some(from)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Some((_, zip)) = zips.iter_mut().find(|(v, _)| *v == version) else {
            return Err(format!("no {version} jar for {dest}"));
        };
        let mut entry = zip.by_name(from).map_err(|e| format!("{from}: {e}"))?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        write(&fresh.join(dest), &bytes)?;
        count += 1;
    }
    log::info!("[shooting_star] the content mod has the original's {count} files");
    let films = record_films(&fresh, cache, &jars);
    // In place of the old copy, whole.
    let old = target.with_file_name(format!(".{MOD_ID}.old"));
    let _ = std::fs::remove_dir_all(&old);
    if target.exists() {
        std::fs::rename(&target, &old).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    std::fs::rename(&fresh, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    let _ = std::fs::remove_dir_all(&old);
    turn_on(game_dir).map_err(|e| e.to_string())?;
    Ok(films)
}

/// Only the films' tables, into the content mod already there.
fn films(game_dir: &Path, cache: &Path) -> Result<bool, String> {
    let jars = inner_jars(cache)?;
    Ok(record_films(&content_dir(game_dir), cache, &jars))
}

fn record_films(content: &Path, cache: &Path, jars: &[(&'static str, Vec<u8>)]) -> bool {
    let Some(java) = find_java() else {
        log::warn!("[shooting_star] no Java 25 found: the films stay off (Horizon Launcher: java install 25)");
        return false;
    };
    set(Status::Working("Recording the films' cameras".to_owned()));
    let tool = cache.join("films-tool");
    for (path, bytes) in TOOL {
        if write(&tool.join(path), bytes).is_err() {
            return false;
        }
    }
    let out = content.join("data").join(MOD_ID).join("films");
    let sep = if cfg!(windows) { ";" } else { ":" };
    for (version, bytes) in jars {
        let jar = cache.join(format!("mc26.3-{version}.jar"));
        if write(&jar, bytes).is_err() {
            return false;
        }
        let mut command = std::process::Command::new(&java);
        command.arg("-cp").arg(format!("{}{sep}{}", tool.display(), jar.display())).arg("Films").arg(&out);
        if *version == "1.4.4" {
            command.arg("--tesseract");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        match command.output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                log::warn!("[shooting_star] the films' recorder failed: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or(""));
                return false;
            }
            Err(e) => {
                log::warn!("[shooting_star] {} could not run: {e}", java.display());
                return false;
            }
        }
    }
    true
}

/// A Java of version 25 or later: JAVA_HOME's, the PATH's, or one of the
/// runtimes Horizon Launcher and the Minecraft Launcher download.
fn find_java() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        candidates.push(PathBuf::from(home).join("bin").join(exe));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|p| p.join(exe)));
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["APPDATA", "LOCALAPPDATA", "XDG_DATA_HOME"] {
        if let Some(base) = std::env::var_os(var).map(PathBuf::from) {
            roots.push(base.join("horizon-launcher").join("minecraft").join("runtime"));
            roots.push(base.join(".minecraft").join("runtime"));
            roots.push(base.join("Packages").join("Microsoft.4297127D64EC6_8wekyb3d8bbwe").join("LocalCache").join("Local").join("runtime"));
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".local/share/horizon-launcher/minecraft/runtime"));
        roots.push(home.join("Library/Application Support/horizon-launcher/minecraft/runtime"));
        roots.push(home.join(".minecraft/runtime"));
        roots.push(home.join("Library/Application Support/minecraft/runtime"));
    }
    for root in roots {
        find_under(&root, exe, 6, &mut candidates);
    }
    candidates.into_iter().find(|java| java.is_file() && java_major(java).is_some_and(|v| v >= 25))
}

fn find_under(dir: &Path, exe: &str, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "bin") && path.join(exe).is_file() {
                out.push(path.join(exe));
            } else if depth > 0 {
                find_under(&path, exe, depth - 1, out);
            }
        }
    }
}

fn java_major(java: &Path) -> Option<u32> {
    let mut command = std::process::Command::new(java);
    command.arg("-version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let out = command.output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let quoted = text.split('"').nth(1)?;
    quoted.split(['.', '-', '+']).next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_names_both_jars_only() {
        let text = std::str::from_utf8(FILES).expect("utf-8");
        let mut lines = 0;
        for line in text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let parts: Vec<&str> = line.split('\t').collect();
            assert_eq!(parts.len(), 3, "{line}");
            assert!(JARS.iter().any(|j| j.version == parts[1]), "{line}");
            assert!(parts[0].starts_with("assets/") || parts[0].starts_with("data/"), "{line}");
            lines += 1;
        }
        assert!(lines > 200);
    }

    #[test]
    fn the_mod_list_line_is_added_once() {
        let dir = std::env::temp_dir().join(format!("shooting-star-setup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("horizon-mods.txt"), "mod.other:true\nmod.shooting_star_demo:false\n").unwrap();
        assert!(turn_on(&dir).unwrap());
        assert!(!turn_on(&dir).unwrap());
        let text = std::fs::read_to_string(dir.join("horizon-mods.txt")).unwrap();
        assert_eq!(text, "mod.other:true\nmod.shooting_star_demo:true\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
