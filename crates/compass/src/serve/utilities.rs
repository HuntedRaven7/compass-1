//! The utilities' data: Browse Fonts' families, the calculator's history
//! database and its exchange rates. The calculator's requests are in
//! [`super::calculator`].

use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

/// Fonts, calculator history and exchange rates.
pub(super) struct Utilities {
    /// Browse Fonts' families, read once.
    pub(super) fonts: Arc<tokio::sync::OnceCell<Vec<compass_core::font_service::BrowsedFamily>>>,
    /// The calculator history's database, once the keyring opened it.
    pub(super) calculator: Arc<tokio::sync::Mutex<Option<crate::extension_runner::Storage>>>,
    /// The calculator's exchange rates.
    pub(super) exchange_rates: Arc<crate::exchange_rates::ExchangeRateService>,
}

impl Utilities {
    /// Nothing read yet, with rates from `exchange_rates`.
    pub(super) fn new(exchange_rates: crate::exchange_rates::ExchangeRateService) -> Self {
        Self {
            fonts: Arc::default(),
            calculator: Arc::default(),
            exchange_rates: Arc::new(exchange_rates),
        }
    }
}

/// The font and exchange rate requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        request @ (Request::ExchangeRates | Request::RefreshExchangeRates) => {
            let rates = Arc::clone(&state.read().await.utilities.exchange_rates);
            rates.answer(request).await
        }
        Request::ListFonts => {
            let fonts = Arc::clone(&state.read().await.utilities.fonts);
            let families = installed_fonts(&fonts).await;
            Response::Fonts {
                fonts: families.iter().map(crate::fonts::entry).collect(),
                categories: crate::fonts::category_names(),
            }
        }
        Request::FontSpecimen { name } => {
            let fonts = Arc::clone(&state.read().await.utilities.fonts);
            match installed_fonts(&fonts)
                .await
                .iter()
                .find(|f| f.name == name)
            {
                Some(family) => Response::Text {
                    text: crate::fonts::specimen(family),
                },
                None => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no installed font family has that name",
                )),
            }
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a utilities request",
        )),
    }
}

/// Browse Fonts' families, read from the font database the first time they
/// are asked for (or warmed at start), then kept.
pub(super) async fn installed_fonts(
    fonts: &tokio::sync::OnceCell<Vec<compass_core::font_service::BrowsedFamily>>,
) -> &[compass_core::font_service::BrowsedFamily] {
    fonts
        .get_or_init(|| async {
            tokio::task::spawn_blocking(crate::fonts::browse)
                .await
                .unwrap_or_default()
        })
        .await
}
