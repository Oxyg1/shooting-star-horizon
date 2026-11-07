// The spells' timelines and Imaginary Space, run by the tick of the level
// they are in, on the server.
fn shooting_star_level_tick_post(
    game: &mut crate::game::native_hooks::Game<'_>,
    dimension: &str,
) {
    if game.native_window() {
        return;
    }
    crate::game::native_hooks::shooting_star_spells::level_tick(game, dimension);
}
