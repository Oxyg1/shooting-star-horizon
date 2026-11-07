// The spell menu's drawing, on every screen the game draws.
fn shooting_star_screen_render_post(
    game: &crate::game::native_hooks::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
) {
    crate::game::native_hooks::shooting_star::menu_render(game, batch, screen);
}
