// A broken chest commits its last changes, empties its slots and keeps its
// frequency for the drop.
fn observe_destroy_block(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    state: BlockStateId,
    canceled: &mut bool,
) {
    crate::game::enderstorage_state::on_destroy_block(game, pos, state, canceled);
}
