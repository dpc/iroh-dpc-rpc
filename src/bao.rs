use bao_tree::io::outboard::{EmptyOutboard, PreOrderMemOutboard};
use bao_tree::io::round_up_to_chunks;
use bao_tree::{BlockSize, ByteRanges};
use convi::CastInto as _;
use iroh_io::{TokioStreamReader, TokioStreamWriter};
use snafu::{OptionExt as _, ResultExt as _};

use crate::{DecodingSnafu, EncodingSnafu, RpcResult, WriteLenSnafu};

#[cfg(test)]
mod test;

impl super::RpcWrite {
    pub async fn write_message_bao(&mut self, bytes: &[u8]) -> RpcResult<bao_tree::blake3::Hash> {
        let bytes_len = u32::try_from(bytes.len()).ok().context(WriteLenSnafu {
            len: u32::MAX,
            limit: u32::MAX,
        })?;
        /// Use a block size of 16 KiB, a good default
        /// for most cases
        const BLOCK_SIZE: BlockSize = BlockSize::from_chunk_log(4);
        let ranges = ByteRanges::from(0u64..bytes_len.into());
        let ranges = round_up_to_chunks(&ranges);
        let mut ob = PreOrderMemOutboard::create(bytes, BLOCK_SIZE);

        bao_tree::io::fsm::encode_ranges_validated(
            bytes,
            &mut ob,
            &ranges,
            TokioStreamWriter(&mut self.send),
        )
        .await
        .boxed()
        .context(EncodingSnafu)?;

        Ok(ob.root)
    }
}

impl super::RpcRead {
    pub async fn read_message_bao(
        &mut self,
        len: u32,
        hash: bao_tree::blake3::Hash,
    ) -> RpcResult<Vec<u8>> {
        const BLOCK_SIZE: BlockSize = BlockSize::from_chunk_log(4);
        let ranges = ByteRanges::from(0u64..len.into());
        let ranges = round_up_to_chunks(&ranges);
        let mut ob = EmptyOutboard {
            tree: bao_tree::BaoTree::new(len.into(), BLOCK_SIZE),
            root: hash,
        };

        let mut decoded = Vec::with_capacity(len.cast_into());
        bao_tree::io::fsm::decode_ranges(
            TokioStreamReader(&mut self.recv),
            ranges,
            &mut decoded,
            &mut ob,
        )
        .await
        .boxed()
        .context(DecodingSnafu)?;

        Ok(decoded)
    }
}
