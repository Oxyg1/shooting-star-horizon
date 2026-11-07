fn shooting_star_render_needs(needs: &mut crate::gfx::native_render::RenderNeeds) {
    // The effects' light needs the world's depth to hide behind the ground.
    needs.world_depth = true;
}
