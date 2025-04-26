use reliquary::network::command::GameCommand;

pub mod database;
pub mod fribbels;

pub trait Exporter {
    type Export;

    fn read_command(&mut self, command: GameCommand);
    fn is_empty(&self) -> bool;
    fn is_finished(&self) -> bool;
    fn export(self) -> Option<Self::Export>;
}
