use reliquary::network::GameCommand;

pub mod fribbels;

pub trait Exporter {
    type Export;
    fn read_command(&mut self, command: GameCommand);
    fn is_finished(&self) -> bool;
    fn export(self) -> Self::Export;
}