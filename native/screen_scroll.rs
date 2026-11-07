// The wheel over the spell menu: the tuning under it moves, or the next skill is picked.
fn shooting_star_screen_scroll(
    game: &crate::game::native_hooks::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    scroll: [f64; 2],
    canceled: &mut bool,
) {
    crate::game::native_hooks::shooting_star::menu_scroll(game, screen, scroll, canceled);
}
