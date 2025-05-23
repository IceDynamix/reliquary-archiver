use reliquary::network::command::GameCommand;

pub mod database;
pub mod fribbels;

pub trait Exporter: Send + 'static {
    type Export: Send;

    fn read_command(&mut self, command: GameCommand);
    fn is_empty(&self) -> bool;
    fn is_initialized(&self) -> bool;
    fn export(&self) -> Option<Self::Export>;

    #[cfg(feature = "stream")]
    type LiveEvent: serde::Serialize + Send + Clone;

    /// Returns a tuple containing an initial export if the exporter is initialized and a broadcast receiver for live events.
    /// 
    /// The first element of the tuple is an optional initial event that represents the current state.
    /// The second element is a broadcast receiver that will receive all future live events.
    /// 
    /// This method is only available when the "stream" feature is enabled.
    #[cfg(feature = "stream")]
    fn subscribe(&self) -> (Option<Self::LiveEvent>, tokio::sync::broadcast::Receiver<Self::LiveEvent>);
}
