use super::super::*;
use lord_commit::list_commitments;

#[derive(Debug, Parser)]
pub(crate) struct List;

impl List {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let entries = list_commitments(settings.data_dir())?;
    Ok(Some(Box::new(entries)))
  }
}
