// The server is stopping: flush the store.
fn observe_server_stopping(game: &mut crate::game::Game<'_>) {
    crate::game::enderstorage_state::on_server_stopping(game);
}
