// `/tss …` on the server: the mod's own commands (info, cast, select, next).
fn shooting_star_perform_command(
    game: &mut crate::game::native_hooks::Game<'_>,
    command: &str,
    ran: &mut bool,
) {
    let Ok(answer) = crate::game::native_hooks::shooting_star::perform(game, command) else {
        return;
    };
    *ran = true;
    game.native_feedback(Text::literal(answer));
}
