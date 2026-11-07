// A message from a player's window (`net_from_client`): the skill they picked
// and how they tuned each one, kept on the server for their casts.
fn shooting_star_net_from_client(
    game: &mut crate::game::native_hooks::Game<'_>,
    player: u128,
    channel: &str,
    data: &[u8],
) {
    let _ = game;
    crate::game::native_hooks::shooting_star_spells::on_client_message(player, channel, data);
}
