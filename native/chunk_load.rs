// A chunk loaded on the server: in a level sent into Imaginary Space, it
// becomes Imaginary Space too (made on a later tick, as the hook asks).
fn shooting_star_chunk_load(
    game: &mut crate::game::native_hooks::Game<'_>,
    dimension: &str,
    chunk: [i32; 2],
    new: bool,
) {
    let _ = (game, new);
    crate::game::native_hooks::shooting_star_city::chunk_loaded(dimension, chunk);
}
