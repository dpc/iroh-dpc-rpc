use snafu::Snafu;

/// Snafu's `Whatever`, but `Send + Sync`
#[derive(Debug, Snafu)]
#[snafu(whatever, display("{message}"))]
pub struct Whatever {
    #[snafu(source(from(Box<dyn std::error::Error + Send + Sync>, Some)))]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,

    message: String,
}
