// The remote's indicator: the selected skill and its recharge, in a corner of
// the screen, while it matters.
fn shooting_star_hud(
    game: &mut crate::game::native_hooks::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    crate::game::native_hooks::shooting_star::hud(game, batch, corners);
}
