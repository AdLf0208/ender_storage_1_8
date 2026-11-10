// An explosion about to break a chest commits and empties it like a break.
fn observe_explosion_detonate(
    game: &mut crate::game::Game<'_>,
    dimension: &str,
    blocks: &mut Vec<[i32; 3]>,
) {
    crate::game::enderstorage_state::on_explosion_detonate(game, dimension, blocks);
}
