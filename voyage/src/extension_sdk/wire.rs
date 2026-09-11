use anyhow::{Result, bail, ensure};
use serde::Serialize;
use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) const MAX_FRAME: usize = 1024 * 1024 - 1; // One byte reserved for LF.

/// Strict JSON including duplicate object keys, with a budget during parsing,
/// not only after an attacker-controlled tree has been allocated.
pub(crate) fn parse_json(bytes: &[u8]) -> Result<Value> {
    ensure!(bytes.len() <= MAX_FRAME, "extension frame exceeds limit");
    let mut nodes = 4096;
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Seed {
        depth: 0,
        nodes: &mut nodes,
    }
    .deserialize(&mut decoder)
    .map_err(|_| anyhow::anyhow!("invalid or excessive extension JSON"))?;
    decoder
        .end()
        .map_err(|_| anyhow::anyhow!("trailing extension JSON"))?;
    Ok(value)
}

/// Preserve duplicate-key/node/depth validation when embedded in a package.
pub(crate) fn deserialize_json<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Value, D::Error> {
    let mut nodes = 4096;
    Seed {
        depth: 0,
        nodes: &mut nodes,
    }
    .deserialize(decoder)
}

struct Seed<'a> {
    depth: usize,
    nodes: &'a mut usize,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        if self.depth > 48 || *self.nodes == 0 {
            return Err(D::Error::custom("JSON limit"));
        }
        *self.nodes -= 1;
        de.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_bool<E: Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E: Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E: Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid number"))
    }
    fn visit_str<E: Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.into()))
    }
    fn visit_string<E: Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_unit<E: Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(Seed {
            depth: self.depth + 1,
            nodes: self.nodes,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate key"));
            }
            let value = map.next_value_seed(Seed {
                depth: self.depth + 1,
                nodes: self.nodes,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

pub(super) async fn read<R: AsyncBufRead + Unpin + ?Sized>(reader: &mut R) -> Result<Value> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            bail!("extension EOF before complete frame");
        }
        let newline = chunk.iter().position(|b| *b == b'\n');
        let take = newline.map_or(chunk.len(), |i| i + 1);
        ensure!(
            bytes.len() + take <= MAX_FRAME + 1,
            "extension frame exceeds limit"
        );
        bytes.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if newline.is_some() {
            break;
        }
    }
    bytes.pop();
    ensure!(
        !bytes.is_empty() && !bytes.contains(&b'\r'),
        "invalid NDJSON framing"
    );
    parse_json(&bytes)
}

pub(super) async fn write<W: AsyncWrite + Unpin + ?Sized, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    parse_json(&bytes)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicates_depth_nodes_and_trailing_data() {
        for bytes in [br#"{"a":1,"a":2}"#.as_slice(), b"{}{}", b"NaN"] {
            assert!(parse_json(bytes).is_err());
        }
        assert!(parse_json(format!("{}0{}", "[".repeat(49), "]".repeat(49)).as_bytes()).is_err());
        assert!(parse_json(format!("[{}0]", "0,".repeat(4096)).as_bytes()).is_err());
        assert!(parse_json(br#"{"ok":[null,true,42,"text"]}"#).is_ok());
    }
    #[tokio::test]
    async fn framing_requires_lf_and_bounds_before_allocation() {
        let mut missing = &b"{}"[..];
        assert!(read(&mut missing).await.is_err());
        let bytes = vec![b'x'; MAX_FRAME + 2];
        assert!(read(&mut bytes.as_slice()).await.is_err());
        let mut two = &b"{}\n[]\n"[..];
        assert!(read(&mut two).await.unwrap().is_object());
        assert!(read(&mut two).await.unwrap().is_array());
    }
}
