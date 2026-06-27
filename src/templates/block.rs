use super::*;

#[derive(Boilerplate)]
pub(crate) struct BlockHtml {
  best_height: Height,
  block: Block,
  hash: BlockHash,
  height: Height,
  target: BlockHash,
}

impl BlockHtml {
  pub(crate) fn new(block: Block, height: Height, best_height: Height) -> Self {
    Self {
      hash: block.header.block_hash(),
      target: target_as_block_hash(block.header.target()),
      block,
      height,
      best_height,
    }
  }
}

impl PageContent for BlockHtml {
  fn title(&self) -> String {
    format!("Block {}", self.height)
  }
}
