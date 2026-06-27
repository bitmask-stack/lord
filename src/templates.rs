use {super::*, boilerplate::Boilerplate};

pub(crate) use {
  crate::subcommand::server::ServerConfig, address::AddressHtml, block::BlockHtml, clock::ClockSvg,
  home::HomeHtml, input::InputHtml, output::OutputHtml, satscard::SatscardHtml,
};

#[cfg(feature = "sats")]
pub(crate) use {rare::RareTxt, sat::SatHtml};

pub use {blocks::BlocksHtml, status::StatusHtml, transaction::TransactionHtml};

pub mod address;
pub mod block;
pub mod blocks;
mod clock;
mod home;
mod input;
pub mod output;
#[cfg(feature = "sats")]
mod rare;
#[cfg(feature = "sats")]
pub mod sat;
mod satscard;
pub mod status;
pub mod transaction;

#[derive(Boilerplate)]
pub struct PageHtml<T: PageContent> {
  content: T,
  config: Arc<ServerConfig>,
}

impl<T> PageHtml<T>
where
  T: PageContent,
{
  pub fn new(content: T, config: Arc<ServerConfig>) -> Self {
    Self { content, config }
  }

  fn og_image(&self) -> String {
    let path = self
      .content
      .og_image_path()
      .unwrap_or_else(|| "/static/favicon.png".into());

    if let Some(domain) = &self.config.domain {
      format!("https://{domain}{path}")
    } else {
      format!("https://ordinals.com{path}")
    }
  }

  fn superscript(&self) -> String {
    if self.config.chain == Chain::Mainnet {
      "beta".into()
    } else {
      self.config.chain.to_string()
    }
  }
}

pub trait PageContent: Display + 'static {
  fn title(&self) -> String;

  fn page(self, server_config: Arc<ServerConfig>) -> PageHtml<Self>
  where
    Self: Sized,
  {
    PageHtml::new(self, server_config)
  }

  fn og_image_path(&self) -> Option<String> {
    None
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  struct Foo;

  impl Display for Foo {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
      write!(f, "<h1>Foo</h1>")
    }
  }

  impl PageContent for Foo {
    fn title(&self) -> String {
      "Foo".to_string()
    }
  }

  #[test]
  fn page() {
    assert_regex_match!(
      Foo.page(Arc::new(ServerConfig {
        chain: Chain::Mainnet,
        csp_origin: Some("https://signet.ordinals.com".into()),
        domain: Some("signet.ordinals.com".into()),
        index_sats: true,
        ..default()
      }),),
      r".*<meta property=og:image content='https://signet.ordinals.com/static/favicon.png'>.*<a href=/ title=home>Lord<sup>beta</sup></a>.*<a href=/blocks title=blocks>.*</a>.*<a href=/rare.txt title=rare>.*</a>.*<form action=/search method=get>.*<h1>Foo</h1>.*"
    );
  }

  #[test]
  fn page_mainnet() {
    assert_regex_match!(
      Foo.page(Arc::new(ServerConfig {
        chain: Chain::Mainnet,
        csp_origin: None,
        domain: None,
        index_sats: true,
        ..default()
      })),
      r".*<nav>\s*<a href=/ title=home>Lord<sup>beta</sup></a>.*"
    );
  }

  #[test]
  fn page_no_sat_index() {
    assert_regex_match!(
      Foo.page(Arc::new(ServerConfig {
        chain: Chain::Mainnet,
        csp_origin: None,
        domain: None,
        index_sats: false,
        ..default()
      })),
      r".*<nav>\s*<a href=/ title=home>Lord<sup>beta</sup></a>.*<a href=/clock title=clock>.*</a>\s*<form action=/search.*",
    );
  }

  #[test]
  fn page_signet() {
    assert_regex_match!(
      Foo.page(Arc::new(ServerConfig {
        chain: Chain::Signet,
        csp_origin: None,
        domain: None,
        index_sats: true,
        ..default()
      })),
      r".*<nav>\s*<a href=/ title=home>Lord<sup>signet</sup></a>.*"
    );
  }
}
