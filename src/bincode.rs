use ::bincode::{Decode, Encode, config};
use snafu::ResultExt as _;

use crate::{
    DecodingSnafu, EncodingSnafu, RpcExt, RpcId, RpcRead, RpcReadResult, RpcResult, RpcWrite,
    RpcWriteResult, WriteSnafu,
};

#[cfg(test)]
mod test;

pub const STANDARD_LIMIT_16M: usize = 0x1_0000_0000;

pub const STD_BINCODE_CONFIG: config::Configuration<
    config::BigEndian,
    config::Varint,
    config::Limit<4294967296>,
> = config::standard()
    .with_limit::<STANDARD_LIMIT_16M>()
    .with_big_endian()
    .with_variable_int_encoding();

fn decode_whole<T>(bytes: &[u8]) -> Result<T, bincode::error::DecodeError>
where
    T: bincode::Decode<()>,
{
    let (v, consumed_len) = bincode::decode_from_slice(bytes, STD_BINCODE_CONFIG)?;

    if consumed_len != bytes.len() {
        return Err(bincode::error::DecodeError::Other(
            "Not all bytes consumed during decoding",
        ));
    }

    Ok(v)
}

impl RpcWrite {
    /// Write a bincode-encoded message
    pub async fn write_message_bincode<T: Encode>(&mut self, v: &T) -> RpcWriteResult<()> {
        let mut bytes = Vec::with_capacity(128);

        ::bincode::encode_into_std_write(v, &mut bytes, STD_BINCODE_CONFIG)
            .boxed()
            .context(EncodingSnafu)?;

        self.send
            .write_all(
                &u32::try_from(bytes.len())
                    .expect("Can't fail")
                    .to_be_bytes(),
            )
            .await
            .boxed()
            .context(WriteSnafu)?;

        self.send
            .write_all(&bytes)
            .await
            .boxed()
            .context(WriteSnafu)?;
        Ok(())
    }
}

impl RpcRead {
    /// Read a bincode-encoded message
    pub async fn read_message_bincode<T: Decode<()>>(&mut self) -> RpcReadResult<T> {
        let bytes = self.read_message_raw().await?;

        decode_whole::<T>(&bytes).boxed().context(DecodingSnafu)
    }
}

/// Extension trait adding bincode-encoded rpc functionality to
/// [`iroh::endpoint::Connection`]
#[async_trait::async_trait]
pub trait RpcExtBincode: RpcExt {
    /// Make a simple request-response rpc, where both messages are encoded
    /// using `bincode`
    async fn make_request_response_bincode<Req, Resp>(
        &mut self,
        rpc_id: impl Into<RpcId> + Send,
        req: Req,
    ) -> RpcResult<Resp>
    where
        Req: Encode + Send + Sync + 'static,
        Resp: ::bincode::Decode<()> + Send;
}

#[async_trait::async_trait]
impl<T> RpcExtBincode for T
where
    T: RpcExt + Send,
{
    async fn make_request_response_bincode<Req, Resp>(
        &mut self,
        rpc_id: impl Into<RpcId> + Send,
        req: Req,
    ) -> RpcResult<Resp>
    where
        Req: Encode + Send + Sync + 'static,
        Resp: ::bincode::Decode<()> + Send,
    {
        self.make_rpc_raw(rpc_id, |mut w, mut r| async move {
            w.write_message_bincode(&req).await?;
            let resp = r.read_message_bincode().await?;
            Ok(resp)
        })
        .await
    }
}
