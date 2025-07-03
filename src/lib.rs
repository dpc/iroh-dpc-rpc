#[cfg(feature = "bao")]
pub mod bao;
#[cfg(feature = "bincode")]
pub mod bincode;
pub mod error;
mod util_error;
use std::fmt;
use std::future::Future;
use std::marker::Sync;
use std::result::Result;
use std::sync::Arc;

use convi::CastInto as _;
use derive_more::From;
use error::Whatever;
use fnv::FnvHashMap;
use futures_lite::FutureExt;
use iroh::PublicKey;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use snafu::{ResultExt as _, Snafu, whatever};
use tracing::debug;
use util_error::BoxedError;

const LOG_TARGET: &str = "iroh-dpc-rpc";
const LIMIT: u32 = 1_000_000;

#[derive(Debug, Snafu)]
pub enum RpcReadError {
    Read { source: BoxedError },
    ReadLen { len: u32, limit: u32 },
    Decoding { source: BoxedError },
}

pub type RpcReadResult<T> = Result<T, RpcReadError>;

pub struct RpcRead {
    recv: RecvStream,
}

impl RpcRead {
    pub async fn read_message_raw(&mut self) -> RpcReadResult<Vec<u8>> {
        let mut len_bytes = [0u8; 4];
        self.recv
            .read_exact(len_bytes.as_mut_slice())
            .await
            .boxed()
            .context(ReadSnafu)?;

        let len = u32::from_be_bytes(len_bytes);

        if LIMIT < len {
            return ReadLenSnafu { len, limit: LIMIT }.fail();
        }

        let len = len.cast_into();

        let mut resp_bytes = vec![0u8; len];

        self.recv
            .read_exact(resp_bytes.as_mut_slice())
            .await
            .boxed()
            .context(ReadSnafu)?;

        Ok(resp_bytes)
    }

    async fn read_request_id(&mut self) -> RpcReadResult<RpcId> {
        let mut id_bytes = [0u8; 2];

        self.recv
            .read_exact(id_bytes.as_mut_slice())
            .await
            .boxed()
            .context(ReadSnafu)?;

        let id = RpcId::from(u16::from_be_bytes(id_bytes));

        Ok(id)
    }
}

#[derive(Debug, Snafu)]
pub enum RpcWriteError {
    Write { source: BoxedError },
    WriteLen { len: u32, limit: u32 },
    Encoding { source: BoxedError },
}

pub type RpcWriteResult<T> = Result<T, RpcWriteError>;

pub const ACK_OK: u8 = 0;
pub const ACK_RPC_ID_NOT_FOUND: u8 = 1;

pub struct RpcWrite {
    send: SendStream,
}

impl RpcWrite {
    async fn write_rpc_id(&mut self, rpc_id: RpcId) -> RpcWriteResult<()> {
        self.send
            .write_all(&rpc_id.0.to_be_bytes())
            .await
            .boxed()
            .context(WriteSnafu)?;
        Ok(())
    }

    pub async fn write_message_raw(&mut self, msg: &[u8]) -> RpcWriteResult<()> {
        self.send
            .write_all(&msg.len().to_be_bytes())
            .await
            .boxed()
            .context(WriteSnafu)?;
        self.send.write_all(msg).await.boxed().context(WriteSnafu)?;
        Ok(())
    }
}

/// Type erased handler fn
type HandlerFn<S> =
    Box<dyn Fn(S, RpcWrite, RpcRead) -> futures_lite::future::Boxed<()> + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, From)]
pub struct RpcId(u16);

impl fmt::Display for RpcId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("{}", self.0))
    }
}

struct DpcRpcInner<S> {
    state: S,
    handlers: FnvHashMap<RpcId, HandlerFn<S>>,
}

#[derive(Clone)]
pub struct DpcRpc<S> {
    inner: Arc<DpcRpcInner<S>>,
}
impl<S> DpcRpc<S>
where
    S: Send + Sync + 'static + Clone,
{
    async fn handle_request(self, send: RpcWrite, recv: RpcRead, remote_node_id: PublicKey) {
        if let Err(err) = self.handle_request_try(send, recv, remote_node_id).await {
            debug!(
                target: LOG_TARGET,
                from = %remote_node_id,
                err = %err,
                "Rpc request handler failed"
            );
        }
    }
    async fn handle_request_try(
        &self,
        send: RpcWrite,
        mut recv: RpcRead,
        remote_node_id: PublicKey,
    ) -> Result<(), Whatever> {
        let rpc_id = recv
            .read_request_id()
            .await
            .whatever_context("Failed to read request id")?;

        debug!(
            target: LOG_TARGET,
            rpc_id = %rpc_id,
            from = %remote_node_id,
            "Rpc request"
        );

        if let Some(handler) = self.inner.handlers.get(&rpc_id) {
            (handler)(self.inner.state.clone(), send, recv).await
        } else {
            whatever!("Request RpcId {rpc_id} not found");
        }
        Ok(())
    }
}

impl<S> fmt::Debug for DpcRpc<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DpcRpc")
    }
}

#[bon::bon]
impl<S> DpcRpc<S> {
    #[builder]
    pub fn new(
        #[builder(start_fn)] state: S,
        #[builder(field)] handlers: FnvHashMap<RpcId, HandlerFn<S>>,
    ) -> Self {
        Self {
            inner: Arc::new(DpcRpcInner { state, handlers }),
        }
    }
}

impl<BS: dpc_rpc_builder::State, S> DpcRpcBuilder<S, BS>
where
    S: Send + Clone,
{
    pub fn handler<F, FF>(mut self, rpc_id: impl Into<RpcId>, handler: F) -> Self
    where
        F: Send + Sync + 'static,
        F: Fn(S, RpcWrite, RpcRead) -> FF,
        FF: Future<Output = ()> + Send + 'static,
    {
        let rpc_id = rpc_id.into();
        if self
            .handlers
            .insert(rpc_id, Box::new(move |s, w, r| handler(s, w, r).boxed()))
            .is_some()
        {
            panic!("Multiple handler registered for rpc_id: {rpc_id:?}")
        }
        self
    }
}

impl<S> ProtocolHandler for DpcRpc<S>
where
    S: Send + Sync + 'static + Clone,
{
    fn accept(
        &self,
        conn: Connection,
    ) -> impl futures_lite::Future<Output = Result<(), AcceptError>> + std::marker::Send {
        let s = self.clone();
        Box::pin(async move {
            let remote_node_id = conn.remote_node_id().map_err(|source| AcceptError::User {
                source: source.into(),
            })?;
            loop {
                let (send, recv) = conn.accept_bi().await?;
                let (send, recv) = (RpcWrite { send }, RpcRead { recv });

                tokio::spawn(s.clone().handle_request(send, recv, remote_node_id));
            }
        })
    }
}

#[derive(Debug, Snafu)]
pub enum RpcError {
    #[snafu(transparent)]
    Read {
        source: RpcReadError,
    },
    #[snafu(transparent)]
    Write {
        source: RpcWriteError,
    },
    StreamConnection {
        source: iroh::endpoint::ConnectionError,
    },
    Other {
        source: BoxedError,
    },
}
pub type RpcResult<T> = Result<T, RpcError>;

/// Extension trait adding rpc functionality to [`iroh::endpoint::Connection`]
#[async_trait::async_trait]
pub trait RpcExt {
    /// Make an rpc with full control over sequence and types of messages being
    /// sent
    async fn make_rpc_raw<F, FF, O>(
        &mut self,
        rpc_id: impl Into<RpcId> + Send,
        handler: F,
    ) -> RpcResult<O>
    where
        F: Send + Sync + 'static,
        F: FnOnce(RpcWrite, RpcRead) -> FF + 'static,
        FF: Future<Output = RpcResult<O>> + Send + 'static;
}

#[async_trait::async_trait]
impl RpcExt for Connection {
    async fn make_rpc_raw<F, FF, O>(
        &mut self,
        rpc_id: impl Into<RpcId> + Send,
        handler: F,
    ) -> RpcResult<O>
    where
        F: Send + Sync + 'static,
        F: FnOnce(RpcWrite, RpcRead) -> FF,
        FF: Future<Output = RpcResult<O>> + Send + 'static,
    {
        let (send, recv) = self.open_bi().await.context(StreamConnectionSnafu)?;
        let (mut send, recv) = (RpcWrite { send }, RpcRead { recv });

        send.write_rpc_id(rpc_id.into()).await?;

        (handler)(send, recv).await
    }
}

#[cfg(test)]
mod tests;
