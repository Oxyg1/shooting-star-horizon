// A player joined: their window is asked for its selection and tuning, and
// told their recharges.
fn shooting_star_player_logged_in(
    game: &mut crate::game::native_hooks::Game<'_>,
    player: u128,
    name: &str,
) {
    let _ = name;
    crate::game::native_hooks::shooting_star_spells::player_joined(game, player);
}
