// Once a tick in every game: the window plays its spells' timelines, the
// server keeps its clock.
fn shooting_star_tick(game: &mut crate::game::native_hooks::Game<'_>) {
    if game.native_window() {
        crate::game::native_hooks::shooting_star::client_tick(game);
    } else {
        crate::game::native_hooks::shooting_star_spells::server_tick(game);
    }
}
