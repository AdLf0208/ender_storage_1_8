// The chest block entity ticks (the store mirror runs there).
fn observe_block_entity_ticks(block: &str, ticks: &mut bool) {
    crate::game::enderstorage_state::on_block_entity_ticks(block, ticks);
}
