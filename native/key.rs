// A press of one of the mod's keys: its skill is selected, the menu key
// cycles the selection.
fn shooting_star_key(game: &mut crate::game::native_hooks::Game<'_>, key: &str, pressed: bool) {
    crate::game::native_hooks::shooting_star::on_key(game, key, pressed);
}
