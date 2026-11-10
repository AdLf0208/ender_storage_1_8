// Runs at the head of `Game::native_block_entity_fluids` (API 54): the tank
// block declares its one 16-bucket tank, which the fluid helpers then read
// and write in the block entity's saved `fluids`.
fn observe_block_entity_fluids(
    block: &str,
    fluids: &mut Option<horizon_client::containers::ModFluidTanks>,
) {
    crate::game::enderstorage_state::on_block_entity_fluids(block, fluids);
}
