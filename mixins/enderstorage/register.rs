// Declare `/enderstorage freq|info`.
fn observe_register(commands: &mut Vec<NativeCommand>) {
    crate::game::enderstorage_state::on_register_commands(commands);
}
