// The HUD steps aside while a film holds the screen (the original's `hideHud`).
fn shooting_star_gui_layer_pre(canceled: &mut bool) {
    if crate::game::native_hooks::shooting_star::film_weight() > 0.35 {
        *canceled = true;
    }
}
