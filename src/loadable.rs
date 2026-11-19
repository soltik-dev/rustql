//! State of a piece of data read from the server in the background.

use crate::db::DbResult;

/// Data read from the server in the background.
#[derive(Debug, Clone, Default)]
pub enum Loadable<T> {
    /// Not requested yet, or invalidated: it will be requested when needed.
    #[default]
    NotLoaded,
    /// Requested. The number identifies the request: if the data is
    /// invalidated meanwhile, the previous request's answer is discarded.
    Loading(u64),
    Ready(T),
    /// Failed. It is not retried on its own: Refresh or F5 request it again.
    Failed(String),
}

impl<T> Loadable<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn needs_load(&self) -> bool {
        matches!(self, Self::NotLoaded)
    }

    /// Applies the answer to request `ticket` if it is still the current one.
    pub fn settle(&mut self, ticket: u64, result: DbResult<T>) {
        if matches!(self, Self::Loading(t) if *t == ticket) {
            *self = match result {
                Ok(value) => Self::Ready(value),
                Err(e) => Self::Failed(e),
            };
        }
    }
}
