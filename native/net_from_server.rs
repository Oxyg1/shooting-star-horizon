// A spell's start, the recharges and the selection reach the window
// (`net_from_server`): it plays each spell's film, sounds and light.
fn shooting_star_net_from_server(
    game: &mut crate::game::native_hooks::Game<'_>,
    channel: &str,
    data: &[u8],
) {
    let _ = game;
    crate::game::native_hooks::shooting_star::on_message(channel, data);
}
