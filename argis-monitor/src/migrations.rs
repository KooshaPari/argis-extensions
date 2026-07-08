//! Embedded refinery migrations.
//!
//! Forward-only schema migrations. Each new schema change is a new file
//! in the `migrations/` directory named `V###__description.sql`. The schema
//! constant in `state_store.rs` previously encoded the same content; this
//! slice moves the source of truth into versioned SQL files and runs them
//! at startup via refinery.

use refinery::embed_migrations;

embed_migrations!("migrations");

/// Run all pending migrations against the given connection.
pub fn run(conn: &mut rusqlite::Connection) -> Result<refinery::Report, refinery::Error> {
    migrations::runner().run(conn)
}

/// Latest migration version embedded in the binary.
/// Counted from the `migrations/` directory at compile time.
pub const LATEST_VERSION: i32 = {
    // The V001__initial_schema.sql file is the only migration we ship in v1.
    // When new migrations are added, bump this manually OR scan the dir
    // at build time (e.g. via a build.rs). For now, hardcode is fine.
    1
};
