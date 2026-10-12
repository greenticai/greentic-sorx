pub mod foundationdb;
#[cfg(feature = "foundationdb")]
pub mod foundationdb_real;
#[cfg(feature = "postgres")]
pub(crate) mod kv;
pub mod memory;
#[cfg(feature = "postgres")]
pub mod postgres;

pub use foundationdb::{FoundationDbProviderAdapter, FoundationDbProviderConfig};
#[cfg(feature = "foundationdb")]
pub use foundationdb_real::FoundationDbStore;
pub use memory::MemoryStoreProvider;
#[cfg(feature = "postgres")]
pub use postgres::{
    CA_FILE_ENV as POSTGRES_CA_FILE_ENV, DEFAULT_URL_ENV as POSTGRES_DEFAULT_URL_ENV,
    PostgresProviderConfig, PostgresStore,
};
