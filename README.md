# dpc's Iroh Rpc

This is a simple RPC library for Iroh.

How it works:

* Every rpc call is its own stream.
* Each stream starts with a `u16` rpc id.
* After that both sides can send one or more length-prefixed messages,
  that might encode payloads they expect.

Support:

* `bincode`-encoded messages
* `bao-tree`-encoded messages (for incremental verification)

Not exactly rocket science.

TODO:

* `cbor` encoding
* configurable msg size limits
* configurable concurrency limits

See [Echo Example](./examples/echo.rs).
