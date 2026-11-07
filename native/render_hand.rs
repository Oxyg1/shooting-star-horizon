// The film's camera is not the player's eye: no hand in front of it.
fn shooting_star_render_hand(canceled: &mut bool) {
    if crate::game::native_hooks::shooting_star::film_weight() > 0.05 {
        *canceled = true;
    }
}
