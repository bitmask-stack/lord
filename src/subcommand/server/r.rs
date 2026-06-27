use super::*;

pub(super) async fn blockhash(
  Extension(index): Extension<Arc<Index>>,
) -> ServerResult<Json<String>> {
  task::block_in_place(|| {
    Ok(Json(
      index
        .block_hash(None)?
        .ok_or_not_found(|| "blockhash")?
        .to_string(),
    ))
  })
}

pub(super) async fn blockhash_at_height(
  Extension(index): Extension<Arc<Index>>,
  Path(height): Path<u32>,
) -> ServerResult<Json<String>> {
  task::block_in_place(|| {
    Ok(Json(
      index
        .block_hash(Some(height))?
        .ok_or_not_found(|| "blockhash")?
        .to_string(),
    ))
  })
}

pub(super) async fn block_hash_from_height_string(
  Extension(index): Extension<Arc<Index>>,
  Path(height): Path<u32>,
) -> ServerResult<String> {
  task::block_in_place(|| {
    Ok(
      index
        .block_hash(Some(height))?
        .ok_or_not_found(|| "blockhash")?
        .to_string(),
    )
  })
}

pub(super) async fn blockhash_string(
  Extension(index): Extension<Arc<Index>>,
) -> ServerResult<String> {
  task::block_in_place(|| {
    Ok(
      index
        .block_hash(None)?
        .ok_or_not_found(|| "blockhash")?
        .to_string(),
    )
  })
}

pub(super) async fn blockheight_string(
  Extension(index): Extension<Arc<Index>>,
) -> ServerResult<String> {
  task::block_in_place(|| {
    Ok(
      index
        .block_height()?
        .ok_or_not_found(|| "blockheight")?
        .0
        .to_string(),
    )
  })
}

pub(super) async fn blockinfo(
  Extension(index): Extension<Arc<Index>>,
  Path(DeserializeFromStr(query)): Path<DeserializeFromStr<query::Block>>,
) -> ServerResult<Json<api::BlockInfo>> {
  task::block_in_place(|| {
    let hash = match query {
      query::Block::Hash(hash) => hash,
      query::Block::Height(height) => index
        .block_hash(Some(height))?
        .ok_or_not_found(|| format!("block {height}"))?,
    };

    let header = index
      .block_header(hash)?
      .ok_or_not_found(|| format!("block {hash}"))?;

    let info = index
      .block_header_info(hash)?
      .ok_or_not_found(|| format!("block {hash}"))?;

    let stats = index
      .block_stats(info.height.try_into().unwrap())?
      .ok_or_not_found(|| format!("block {hash}"))?;

    Ok(Json(api::BlockInfo {
      average_fee: stats.avg_fee.to_sat(),
      average_fee_rate: stats.avg_fee_rate.to_sat(),
      bits: header.bits.to_consensus(),
      chainwork: info.chainwork.try_into().unwrap(),
      confirmations: info.confirmations,
      difficulty: info.difficulty,
      hash,
      feerate_percentiles: [
        stats.fee_rate_percentiles.fr_10th.to_sat(),
        stats.fee_rate_percentiles.fr_25th.to_sat(),
        stats.fee_rate_percentiles.fr_50th.to_sat(),
        stats.fee_rate_percentiles.fr_75th.to_sat(),
        stats.fee_rate_percentiles.fr_90th.to_sat(),
      ],
      height: info.height.try_into().unwrap(),
      max_fee: stats.max_fee.to_sat(),
      max_fee_rate: stats.max_fee_rate.to_sat(),
      max_tx_size: stats.max_tx_size,
      median_fee: stats.median_fee.to_sat(),
      median_time: info
        .median_time
        .map(|median_time| median_time.try_into().unwrap()),
      merkle_root: info.merkle_root,
      min_fee: stats.min_fee.to_sat(),
      min_fee_rate: stats.min_fee_rate.to_sat(),
      next_block: info.next_block_hash,
      nonce: info.nonce,
      previous_block: info.previous_block_hash,
      subsidy: stats.subsidy.to_sat(),
      target: target_as_block_hash(header.target()),
      timestamp: info.time.try_into().unwrap(),
      total_fee: stats.total_fee.to_sat(),
      total_size: stats.total_size,
      total_weight: stats.total_weight,
      transaction_count: info.n_tx.try_into().unwrap(),
      #[allow(clippy::cast_sign_loss)]
      version: info.version.to_consensus() as u32,
    }))
  })
}

pub(super) async fn blocktime_string(
  Extension(index): Extension<Arc<Index>>,
) -> ServerResult<String> {
  task::block_in_place(|| {
    Ok(
      index
        .block_time(index.block_height()?.ok_or_not_found(|| "blocktime")?)?
        .unix_timestamp()
        .to_string(),
    )
  })
}

pub(super) async fn sat(
  Extension(index): Extension<Arc<Index>>,
  Path(sat): Path<u64>,
) -> ServerResult<Json<api::Sat>> {
  task::block_in_place(|| {
    if !index.has_sat_index() {
      return Err(ServerError::NotFound("this server has no sat index".into()));
    }

    let sat = Sat(sat);
    let satpoint = index.rare_sat_satpoint(sat)?;
    let blocktime = index.block_time(sat.height())?;
    let _block = index.block_header_at_height(sat.height())?;
    let charms = sat.charms();

    let address = if let Some(satpoint) = satpoint {
      if satpoint.outpoint == unbound_outpoint() {
        None
      } else {
        let tx = index
          .get_transaction(satpoint.outpoint.txid)?
          .context("could not get transaction for sat")?;

        let tx_out = tx
          .output
          .get::<usize>(satpoint.outpoint.vout.try_into().unwrap())
          .context("could not get vout for sat")?;

        index
          .as_ref()
          .chain()
          .address_from_script(&tx_out.script_pubkey)
          .ok()
      }
    } else {
      None
    };

    Ok(Json(api::Sat {
      address: address.map(|address| address.to_string()),
      block: sat.height().0,
      charms: Charm::charms(charms),
      cycle: sat.cycle(),
      decimal: sat.decimal().to_string(),
      degree: sat.degree().to_string(),
      epoch: sat.epoch().0,
      name: sat.name(),
      number: sat.0,
      offset: sat.third(),
      percentile: sat.percentile(),
      period: sat.period(),
      rarity: sat.rarity(),
      satpoint,
      timestamp: blocktime.timestamp().timestamp(),
    }))
  })
}

pub(super) async fn sat_paginated(
  index: Extension<Arc<Index>>,
  Path((sat_number, _page)): Path<(u64, u64)>,
) -> ServerResult<Json<api::Sat>> {
  sat(index, Path(sat_number)).await
}

pub(super) async fn tx(
  Extension(index): Extension<Arc<Index>>,
  Path(txid): Path<Txid>,
) -> ServerResult<Json<String>> {
  task::block_in_place(|| {
    Ok(Json(
      index
        .get_transaction_hex_recursive(txid)?
        .ok_or_not_found(|| format!("transaction {txid}"))?,
    ))
  })
}

pub(super) async fn utxo(
  Extension(index): Extension<Arc<Index>>,
  Path(outpoint): Path<OutPoint>,
) -> ServerResult {
  task::block_in_place(|| {
    Ok(
      Json(
        index
          .get_utxo_recursive(outpoint)?
          .ok_or_not_found(|| format!("output {outpoint}"))?,
      )
      .into_response(),
    )
  })
}
