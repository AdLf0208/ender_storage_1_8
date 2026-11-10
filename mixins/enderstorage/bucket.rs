// Runs at the head of `Game::native_bucket_empty` (API 179): a server player
// pours a bucket at `pos`. When `pos` is the tank (its cauldron template is a
// liquid container, so a water bucket targets the block itself) the tank owns
// the click: one bucket goes into its shared tank, and nothing is poured.
fn observe_bucket_empty(
    game: &mut crate::game::Game<'_>,
    player: u128,
    name: &str,
    pos: [i32; 3],
    fluid: &str,
    cancel: &mut bool,
) {
    let _ = (player, name);
    crate::game::enderstorage_state::on_bucket_empty(game, pos, fluid, cancel);
}
