// The mod's page in Options > Mods: its name, what it does and its keys.
fn shooting_star_mods(mods: &mut Vec<NativeMod>) {
    let keys = crate::game::native_hooks::shooting_star::KEYS
        .iter()
        .map(|(name, label, default)| [(*name).to_owned(), (*label).to_owned(), (*default).to_owned()])
        .collect();
    mods.push(NativeMod {
        id: crate::game::native_hooks::shooting_star::ID.to_owned(),
        title: "The Shooting Star (Demo) 1.5.2: Horizon port".to_owned(),
        description: "The Stellar Remote and its five skills, on the original's timelines and sizes: \
                      SS-01 Railgun, SS-03 Gungnir, SS-04 Seven Stars, SS-05 Tesseract and SS-06 Gravitia."
            .to_owned(),
        keys,
    });
}
