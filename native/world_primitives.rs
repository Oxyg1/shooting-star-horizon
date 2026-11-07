// Once a frame while the world is drawn: the spells' passes for the render
// pass, on their own clocks, and the marks of the spells drawn with lines.
fn shooting_star_world_primitives(
    game: &crate::game::native_hooks::Game<'_>,
    partial: f32,
    camera: [f64; 3],
    primitives: &mut horizon_client::world_primitives::WorldPrimitives,
) {
    crate::game::native_hooks::shooting_star::frame_tick(game, partial, camera, primitives);
}
