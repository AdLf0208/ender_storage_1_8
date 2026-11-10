// A dye on the chest sets the next colour band; a bucket on the tank moves a
// bucket of fluid.
fn observe_right_click_block(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    face: [i32; 3],
    canceled: &mut bool,
) {
    crate::game::enderstorage_state::on_right_click_block(game, pos, face, canceled);
}
