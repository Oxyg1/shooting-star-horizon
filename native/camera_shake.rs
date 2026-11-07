// The camera's jolt while a strike still rings (`camera_shake`): the hook
// adds to the shake the game already has, so nothing replaces a vanilla jolt.
fn shooting_star_camera_shake(
    game: &crate::game::native_hooks::Game<'_>,
    partial: f32,
    shake: &mut crate::game::native_hooks::NativeShake,
) {
    crate::game::native_hooks::shooting_star::shake(game, partial, shake);
}
