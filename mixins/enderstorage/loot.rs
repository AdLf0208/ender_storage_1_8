// The broken chest's item drop carries its frequency.
fn observe_loot_drops(
    game: &mut crate::game::Game<'_>,
    context: &NativeLootDrops<'_>,
    drops: &mut Vec<ItemStack>,
) {
    crate::game::enderstorage_state::on_loot_drops(game, context, drops);
}
