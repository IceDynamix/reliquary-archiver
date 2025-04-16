use reliquary::network::command::GameCommand;
use serde::Serialize;

#[cfg(feature = "stream")]
use crate::websocket;

pub mod database;
pub mod fribbels;

pub trait Exporter: Send + 'static {
    type Export: Send;
    type LiveEvent: Send + Serialize;

    fn read_command(&mut self, command: GameCommand);
    fn is_empty(&self) -> bool;
    fn is_finished(&self) -> bool;
    fn export(&self) -> Option<Self::Export>;

    fn get_initial_event(&self) -> Option<Self::LiveEvent>;

    #[cfg(feature = "stream")]
    fn set_streamer(&mut self, _tx: Option<websocket::ClientSender>) {
        // Default implementation that does nothing
    }
}
