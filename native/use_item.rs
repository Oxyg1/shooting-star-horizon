// The Stellar Remote's right click, on the server: cast the selected skill,
// or on a sneak + right click cycle to the next one, as the original does.
fn shooting_star_use_item(
    game: &mut crate::game::native_hooks::Game<'_>,
    stack: &ItemStack,
    claimed: &mut bool,
) {
    if !crate::game::native_hooks::shooting_star::is_remote(&stack.item) {
        return;
    }
    // The remote is not a vanilla item: its right click is ours alone.
    *claimed = true;
    crate::game::native_hooks::shooting_star_spells::use_remote(game);
}
