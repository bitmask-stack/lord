use std::borrow::Cow;
use std::marker::PhantomData;

use heed3::{BoxedError, BytesDecode, BytesEncode};
use rkyv::{
  Archive, Archived, Portable, Serialize,
  api::high::{HighSerializer, HighValidator},
  bytecheck::CheckBytes,
  rancor::Error as RkyvError,
  ser::allocator::ArenaHandle,
  util::AlignedVec,
};

/// Zero-copy heed3 codec for rkyv-archived types.
///
/// Decoded values are `&'a Archived<T>` tied to the input byte slice lifetime `'a`.
/// In heed3, that slice is the value returned from a read transaction, so the archived
/// reference is only valid for as long as the enclosing `heed3::RoTxn` (or `RwTxn`)
/// remains open. Do not use decoded references after the transaction commits or drops.
#[derive(Debug, Default)]
pub struct RkyvCodec<T>(PhantomData<T>);

impl<'a, T> BytesEncode<'a> for RkyvCodec<T>
where
  T: Archive + 'a,
  for<'b> T: Serialize<HighSerializer<AlignedVec, ArenaHandle<'b>, RkyvError>>,
{
  type EItem = T;

  fn bytes_encode(item: &Self::EItem) -> Result<Cow<'_, [u8]>, BoxedError> {
    let bytes = rkyv::to_bytes::<RkyvError>(item).map_err(|e| Box::new(e) as BoxedError)?;
    Ok(Cow::Owned(bytes.into_vec()))
  }
}

impl<'a, T> BytesDecode<'a> for RkyvCodec<T>
where
  T: Archive,
  Archived<T>: Portable + for<'b> CheckBytes<HighValidator<'b, RkyvError>> + 'a,
{
  type DItem = &'a Archived<T>;

  fn bytes_decode(bytes: &'a [u8]) -> Result<Self::DItem, BoxedError> {
    let archived = rkyv::api::high::access::<Archived<T>, RkyvError>(bytes)
      .map_err(|e| Box::new(e) as BoxedError)?;
    Ok(archived)
  }
}
