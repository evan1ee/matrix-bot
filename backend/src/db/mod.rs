#[cfg(test)]
pub mod catalog;
pub mod dialect;
pub mod executor;
pub mod migrate;
pub mod pool;
pub mod rooms;

pub use executor::{DbPool, DbTx};
pub use migrate::run_migrations;
pub use pool::Database;
pub use rooms::{
    get_cached_rooms, resolve_target_room, resolve_user_default_room, save_cached_rooms,
    JoinedRoomInfo, RoomRepository,
};
