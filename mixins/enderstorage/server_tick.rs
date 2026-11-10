// End of a running tick: debounce the store write and re-send changed hands.
fn observe_server_tick(game: &mut crate::game::Game<'_>, phase: ServerTickPhase) {
    crate::game::enderstorage_state::on_server_tick(game, phase);
}
