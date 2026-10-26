//! Estado de un dato que se lee del servidor en segundo plano.

use crate::db::DbResult;

/// Dato que se lee del servidor en segundo plano.
#[derive(Debug, Clone, Default)]
pub enum Loadable<T> {
    /// Sin pedir todavía, o invalidado: se pedirá cuando haga falta.
    #[default]
    NotLoaded,
    /// Pedido. El número identifica la petición: si el dato se invalida
    /// entretanto, la respuesta de la anterior se descarta.
    Loading(u64),
    Ready(T),
    /// Falló. No se reintenta solo: Refrescar o F5 lo vuelven a pedir.
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

    /// Aplica la respuesta a la petición `ticket` si sigue siendo la vigente.
    pub fn settle(&mut self, ticket: u64, result: DbResult<T>) {
        if matches!(self, Self::Loading(t) if *t == ticket) {
            *self = match result {
                Ok(value) => Self::Ready(value),
                Err(e) => Self::Failed(e),
            };
        }
    }
}
