// The film's lens: the shot's field of view, blended with the player's.
fn shooting_star_compute_fov(game: &crate::game::native_hooks::Game<'_>, degrees: &mut f32) {
    let _ = game;
    crate::game::native_hooks::shooting_star::compute_fov(degrees);
}
