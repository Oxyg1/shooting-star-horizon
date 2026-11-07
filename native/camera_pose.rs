// The film's camera, every world frame: the shot of the original's cutscene,
// blended with where vanilla put the camera.
fn shooting_star_camera_pose(
    game: &crate::game::native_hooks::Game<'_>,
    partial: f32,
    position: &mut [f64; 3],
    yaw: &mut f32,
    pitch: &mut f32,
) {
    crate::game::native_hooks::shooting_star::camera_pose(game, partial, position, yaw, pitch);
}
