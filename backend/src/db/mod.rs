pub mod backfill;
pub mod ddl;
pub mod dialect;
pub mod dialect_probe;
pub mod executor;
pub mod pool;
pub mod rooms;
pub mod schema;
pub mod schema_ai;
pub mod schema_auth;
pub mod schema_bills;
pub mod schema_bookmarks;
pub mod schema_finance;
pub mod schema_footprints;
pub mod schema_habits;
pub mod schema_life;
pub mod schema_mcp;
pub mod schema_portfolio;
pub mod schema_reactions;
pub mod schema_reminders;
pub mod schema_rooms;
pub mod schema_settings;
pub mod schema_todos;
pub mod schema_webhooks;

pub use dialect::Dialect;
pub use executor::DbPool;
pub use pool::Database;
pub use rooms::{
    get_cached_rooms, resolve_target_room, resolve_user_default_room, save_cached_rooms,
    JoinedRoomInfo, RoomRepository,
};
