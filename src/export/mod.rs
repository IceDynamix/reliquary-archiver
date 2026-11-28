use reliquary::network::command::GameCommand;
use reliquary::network::command::proto::GachaInfo::GachaInfo;

pub mod database;
pub mod fribbels;

pub trait Exporter: Send + 'static {
    type Export: Send;

    /// A change to the exported data, for clients that follow along live
    type LiveEvent: serde::Serialize + Send + Clone;

    /// Reads a command and returns the live events it makes. There are none until the exporter is
    /// initialized, the first one after that is the whole export.
    fn read_command(&mut self, command: GameCommand) -> Vec<Self::LiveEvent>;
    fn is_empty(&self) -> bool;
    fn is_initialized(&self) -> bool;
    fn export(&self) -> Option<Self::Export>;
}

/// A banner's rate-up 5 and 4 stars, sorted. The standard banner's is its whole pool
pub fn banner_rate_up(banner: &GachaInfo) -> Vec<u32> {
    let mut rate_up = banner.prize_item_list.clone();
    rate_up.sort_unstable();
    rate_up.dedup();
    rate_up
}
