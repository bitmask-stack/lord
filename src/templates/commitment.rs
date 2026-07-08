use super::*;

pub(crate) struct CommitmentHtml {
  pub(crate) bao_root: String,
  pub(crate) carbonado_path: String,
  pub(crate) format: u8,
  pub(crate) visibility: String,
  pub(crate) layout: String,
  pub(crate) filepack_fp: Option<String>,
  pub(crate) created_at: u64,
  pub(crate) ots_proof_path: Option<String>,
  pub(crate) ots_order_key: Option<String>,
  pub(crate) timestamped_at: Option<u64>,
  pub(crate) timestamped: bool,
  pub(crate) ots_attestation: Option<lord_commit::AttestationVerifyStatusJson>,
  pub(crate) attestation_height: Option<u32>,
  pub(crate) attestation_txid: Option<String>,
}

impl Display for CommitmentHtml {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    writeln!(f, "<h1>Commitment</h1>")?;
    writeln!(f, "<dl>")?;
    writeln!(f, "<dt>bao root</dt><dd>{}</dd>", self.bao_root)?;
    writeln!(f, "<dt>carbonado path</dt><dd>{}</dd>", self.carbonado_path)?;
    writeln!(f, "<dt>format</dt><dd>c{}</dd>", self.format)?;
    writeln!(f, "<dt>visibility</dt><dd>{}</dd>", self.visibility)?;
    writeln!(f, "<dt>layout</dt><dd>{}</dd>", self.layout)?;
    if let Some(fp) = &self.filepack_fp {
      writeln!(f, "<dt>filepack</dt><dd>{fp}</dd>")?;
    }
    writeln!(f, "<dt>created at</dt><dd>{}</dd>", self.created_at)?;
    if self.timestamped {
      writeln!(f, "<dt>OTS</dt><dd>timestamped</dd>")?;
      if let Some(ts) = self.timestamped_at {
        writeln!(f, "<dt>timestamped at</dt><dd>{ts}</dd>")?;
      }
      if let Some(key) = &self.ots_order_key {
        writeln!(f, "<dt>order key</dt><dd>{key}</dd>")?;
      }
      if let Some(path) = &self.ots_proof_path {
        writeln!(f, "<dt>proof</dt><dd>{path}</dd>")?;
      }
      if let Some(attestation) = &self.ots_attestation {
        match attestation {
          lord_commit::AttestationVerifyStatusJson::Confirmed {
            height,
            confirmations,
          } => {
            writeln!(f, "<dt>attestation</dt><dd>confirmed</dd>")?;
            if let Some(breccia_height) = self.attestation_height {
              writeln!(
                f,
                "<dt>breccia attestation height</dt><dd>{breccia_height}</dd>"
              )?;
            }
            if let Some(txid) = &self.attestation_txid {
              writeln!(f, "<dt>breccia attestation txid</dt><dd>{txid}</dd>")?;
            }
            writeln!(f, "<dt>block height</dt><dd>{height}</dd>")?;
            if let Some(confirmations) = confirmations {
              writeln!(f, "<dt>confirmations</dt><dd>{confirmations}</dd>")?;
            }
            writeln!(
              f,
              "<p><small>1-conf / best-chain view; reorgs can invalidate attestation.</small></p>"
            )?;
          }
          lord_commit::AttestationVerifyStatusJson::Pending => {
            writeln!(f, "<dt>attestation</dt><dd>pending</dd>")?;
          }
          lord_commit::AttestationVerifyStatusJson::Failed { reason } => {
            writeln!(f, "<dt>attestation</dt><dd>failed ({reason})</dd>")?;
          }
          lord_commit::AttestationVerifyStatusJson::Unavailable { reason } => {
            writeln!(f, "<dt>attestation</dt><dd>unavailable ({reason})</dd>")?;
          }
          lord_commit::AttestationVerifyStatusJson::Unknown => {
            writeln!(f, "<dt>attestation</dt><dd>unknown</dd>")?;
          }
          lord_commit::AttestationVerifyStatusJson::None => {}
        }
      }
    } else {
      writeln!(f, "<dt>OTS</dt><dd>not timestamped</dd>")?;
    }
    writeln!(
      f,
      "<p><a href=/content/{}>content</a> · <a href=/commitments>all commitments</a></p>",
      self.bao_root
    )?;
    write!(f, "</dl>")
  }
}

impl PageContent for CommitmentHtml {
  fn title(&self) -> String {
    format!("Commitment {}", self.bao_root)
  }
}

pub(crate) struct CommitmentsHtml {
  pub(crate) entries: Vec<CommitmentListItem>,
  pub(crate) page: usize,
  pub(crate) total_pages: usize,
}

#[derive(Clone)]
pub(crate) struct CommitmentListItem {
  pub(crate) bao_root: String,
  pub(crate) ots_order_key: String,
  pub(crate) carbonado_path: String,
  pub(crate) format: u8,
  pub(crate) timestamped_at: Option<u64>,
}

impl Display for CommitmentsHtml {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    writeln!(f, "<h1>Commitments</h1>")?;
    writeln!(f, "<p>page {} of {}</p>", self.page, self.total_pages)?;
    writeln!(f, "<ol>")?;
    for entry in &self.entries {
      writeln!(
        f,
        "<li><a href=/commitment/{}>{}</a> (c{}, order {})</li>",
        entry.bao_root, entry.bao_root, entry.format, entry.ots_order_key
      )?;
    }
    writeln!(f, "</ol>")?;
    if self.page > 0 {
      writeln!(
        f,
        "<p><a href=/commitments/{}>previous</a></p>",
        self.page - 1
      )?;
    }
    if self.page + 1 < self.total_pages {
      writeln!(f, "<p><a href=/commitments/{}>next</a></p>", self.page + 1)?;
    }
    Ok(())
  }
}

impl PageContent for CommitmentsHtml {
  fn title(&self) -> String {
    "Commitments".to_string()
  }
}
