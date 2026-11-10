// A chest placed from an item takes the item's frequency.
fn observe_place_block(
    game: &mut crate::game::Game<'_>,
    stack: &horizon_client::items::ItemStack,
    pos: [i32; 3],
    state: BlockStateId,
) {
    crate::game::enderstorage_state::on_place_block(game, stack, pos, state);
}
