// Before content loads: register `enderstorage:frequency` for item stacks.
fn register_component_types(types: &mut Vec<String>) {
    crate::game::enderstorage_state::register_component_types(types);
}
