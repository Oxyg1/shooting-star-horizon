// A click while a screen is up: the spell menu takes its own.
fn shooting_star_screen_click(
    game: &crate::game::native_hooks::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    crate::game::native_hooks::shooting_star::menu_click(game, screen, button, canceled);
}
