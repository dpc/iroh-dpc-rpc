use bincode::config;

pub const STANDARD_LIMIT_16M: usize = 0x1_0000_0000;

pub const STD_BINCODE_CONFIG: config::Configuration<
    config::BigEndian,
    config::Varint,
    config::Limit<4294967296>,
> = config::standard()
    .with_limit::<STANDARD_LIMIT_16M>()
    .with_big_endian()
    .with_variable_int_encoding();

pub(crate) fn decode_whole<T>(bytes: &[u8]) -> Result<T, bincode::error::DecodeError>
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
