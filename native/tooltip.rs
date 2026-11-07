// The Stellar Remote's tooltip: the skill it will cast and the rest of its
// spells, so the item explains itself.
fn shooting_star_tooltip(
    game: &crate::game::native_hooks::Game<'_>,
    stack: &ItemStack,
    lines: &mut Vec<(Text, u32)>,
) {
    crate::game::native_hooks::shooting_star::tooltip(game, stack, lines);
}
