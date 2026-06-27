use super::*;

#[derive(Boilerplate)]
pub(crate) struct HomeHtml;

impl PageContent for HomeHtml {
  fn title(&self) -> String {
    "Lord".to_string()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn html() {
    assert_eq!(HomeHtml.to_string(), "<h1>Lord Explorer</h1>");
  }
}
