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

/// The items a banner features, sorted.
///
/// `prize_item_list` holds every featured item (the whole pool on the standard banner) and
/// `item_detail_list` at most an item that can't be pulled. The field names are reconstructed and
/// could trade places with a proto update, so both are read.
pub fn banner_rate_up(banner: &GachaInfo) -> Vec<u32> {
    let mut rate_up: Vec<u32> = banner.prize_item_list.iter().chain(&banner.item_detail_list).copied().collect();
    rate_up.sort_unstable();
    rate_up.dedup();
    rate_up
}
