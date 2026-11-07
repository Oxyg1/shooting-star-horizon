// The Shooting Star's command usages, pushed when a world opens to players.
fn shooting_star_register_commands(commands: &mut Vec<NativeCommand>) {
    for usage in crate::game::native_hooks::shooting_star::COMMANDS {
        commands.push(NativeCommand::new(usage));
    }
}
