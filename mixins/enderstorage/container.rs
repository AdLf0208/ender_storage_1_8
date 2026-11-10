// The chest block entity has 27 slots and the vanilla 9x3 menu.
fn observe_block_entity_container(
    block: &str,
    container: &mut Option<horizon_client::containers::ModContainer>,
) {
    crate::game::enderstorage_state::on_block_entity_container(block, container);
}
