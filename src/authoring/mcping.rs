//! Asking a Minecraft server what it advertises.
//!
//! The handshake spoof has to claim exactly the mod list the server expects
//! (#110), and the server states that list itself: on 1.12.2 Forge the FML
//! handshake's mod list also rides in the status ping response, and newer Forge
//! carries an equivalent under `forgeData`. So the question "what does this
//! server expect" is a status query -- no account, no login, nothing a player
//! could not do from the multiplayer screen -- and it can be repeated to notice
//! a bump the moment it happens.
//!
//! Only the status path is implemented. A login handshake would need credentials
//! and would join the server to ask it a question, which is a different act.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use ts_rs::TS;

/// The protocol version sent in the handshake. Status is version-agnostic in
/// practice -- a server answers a ping from any protocol -- so this is the
/// conventional "unknown" rather than a claim about the client.
const PROTOCOL_UNKNOWN: i32 = -1;
const STATE_STATUS: i32 = 1;
/// A server that has not answered by now is down as far as this is concerned.
const TIMEOUT: Duration = Duration::from_secs(6);
/// Status responses are small; a larger one is a server saying something this
/// does not understand, and reading it whole would be the only cost.
const MAX_RESPONSE: usize = 4 * 1024 * 1024;

/// One mod a server says it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ServerMod {
    pub modid: String,
    #[serde(default)]
    pub version: String,
}

/// What a server advertised, and what it did not.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct ServerStatus {
    /// The server's own version string, as shown in the multiplayer list.
    pub version: String,
    /// The mod list the server advertises. Empty for a vanilla server, and also
    /// empty for a modded one configured not to advertise -- `advertises_mods`
    /// separates those two, because "no mods" and "will not say" are different
    /// answers and a spoof built from the second would be a guess.
    pub mods: Vec<ServerMod>,
    pub advertises_mods: bool,
}

/// Ask `host:port` what it is running.
pub async fn status(host: &str, port: u16) -> Result<ServerStatus> {
    let mut stream = tokio::time::timeout(TIMEOUT, TcpStream::connect((host, port)))
        .await
        .context("connecting timed out")?
        .with_context(|| format!("connecting to {host}:{port}"))?;

    let mut handshake = Vec::new();
    write_varint(&mut handshake, 0x00); // packet id: handshake
    write_varint(&mut handshake, PROTOCOL_UNKNOWN);
    write_string(&mut handshake, host);
    handshake.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut handshake, STATE_STATUS);
    write_packet(&mut stream, &handshake).await?;

    let mut request = Vec::new();
    write_varint(&mut request, 0x00); // packet id: status request
    write_packet(&mut stream, &request).await?;

    let body = tokio::time::timeout(TIMEOUT, read_packet(&mut stream))
        .await
        .context("the server did not answer in time")??;
    let mut cursor = &body[..];
    let id = read_varint(&mut cursor)?;
    if id != 0x00 {
        bail!("expected a status response, got packet {id}");
    }
    let json = read_string(&mut cursor)?;
    parse_status(&json)
}

/// Pull the version and the advertised mod list out of a status response.
///
/// Two shapes, because Forge changed it: FML1 (1.12.2 and older) puts
/// `modinfo.modList` with `modid`/`version` per entry; FML2 and newer put
/// `forgeData.mods`, where the field is `modId` and the version sits under
/// `version` or is elided entirely for a mod that ships none.
pub fn parse_status(json: &str) -> Result<ServerStatus> {
    let v: serde_json::Value = serde_json::from_str(json).context("status response is not JSON")?;
    let version = v
        .pointer("/version/name")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();

    if let Some(list) = v.pointer("/modinfo/modList").and_then(|x| x.as_array()) {
        return Ok(ServerStatus {
            version,
            mods: list.iter().filter_map(fml1_mod).collect(),
            advertises_mods: true,
        });
    }
    if let Some(list) = v.pointer("/forgeData/mods").and_then(|x| x.as_array()) {
        return Ok(ServerStatus {
            version,
            mods: list.iter().filter_map(fml2_mod).collect(),
            advertises_mods: true,
        });
    }
    Ok(ServerStatus {
        version,
        mods: Vec::new(),
        advertises_mods: false,
    })
}

fn fml1_mod(v: &serde_json::Value) -> Option<ServerMod> {
    Some(ServerMod {
        modid: v.get("modid")?.as_str()?.to_string(),
        version: v
            .get("version")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

fn fml2_mod(v: &serde_json::Value) -> Option<ServerMod> {
    Some(ServerMod {
        modid: v.get("modId")?.as_str()?.to_string(),
        version: v
            .get("version")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

// ── the wire ────────────────────────────────────────────────────────────────
//
// Minecraft frames every packet as a length-prefixed body, and both the length
// and the string lengths are LEB128 varints. Three helpers cover the whole of
// what a status query needs.

fn write_varint(out: &mut Vec<u8>, value: i32) {
    // as u32 rather than a sign-extended i64: the protocol's varint is the
    // two's-complement bit pattern, so -1 is five bytes of ones, not a short
    // negative number
    let mut v = value as u32;
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    write_varint(out, s.len() as i32);
    out.extend_from_slice(s.as_bytes());
}

async fn write_packet(stream: &mut TcpStream, body: &[u8]) -> Result<()> {
    let mut framed = Vec::with_capacity(body.len() + 5);
    write_varint(&mut framed, body.len() as i32);
    framed.extend_from_slice(body);
    stream.write_all(&framed).await.context("writing packet")?;
    Ok(())
}

fn read_varint(buf: &mut &[u8]) -> Result<i32> {
    let mut result: i32 = 0;
    for shift in 0..5 {
        let (&byte, rest) = buf.split_first().context("varint ran off the end")?;
        *buf = rest;
        result |= ((byte & 0x7f) as i32) << (shift * 7);
        if byte & 0x80 == 0 {
            return Ok(result);
        }
    }
    bail!("varint longer than five bytes")
}

fn read_string(buf: &mut &[u8]) -> Result<String> {
    let len = read_varint(buf)? as usize;
    if len > buf.len() {
        bail!("string claims {len} bytes, {} remain", buf.len());
    }
    let (s, rest) = buf.split_at(len);
    *buf = rest;
    String::from_utf8(s.to_vec()).context("status response is not UTF-8")
}

/// Read one length-prefixed packet. The length varint arrives a byte at a time
/// because its own length is not known until it ends.
async fn read_packet(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len_bytes = Vec::with_capacity(5);
    loop {
        let mut b = [0u8; 1];
        stream.read_exact(&mut b).await.context("reading length")?;
        len_bytes.push(b[0]);
        if b[0] & 0x80 == 0 {
            break;
        }
        if len_bytes.len() == 5 {
            bail!("packet length varint longer than five bytes");
        }
    }
    let mut slice = &len_bytes[..];
    let len = read_varint(&mut slice)? as usize;
    if len > MAX_RESPONSE {
        bail!("status response claims {len} bytes");
    }
    let mut body = vec![0u8; len];
    stream
        .read_exact(&mut body)
        .await
        .context("reading the status response")?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip_the_values_the_protocol_uses() {
        for v in [0, 1, 127, 128, 255, 25565, i32::MAX, -1] {
            let mut out = Vec::new();
            write_varint(&mut out, v);
            let mut slice = &out[..];
            assert_eq!(read_varint(&mut slice).unwrap(), v, "value {v}");
            assert!(slice.is_empty(), "value {v} left bytes behind");
        }
    }

    // 1.12.2 Forge: the FML handshake's list, as the ping carries it.
    #[test]
    fn fml1_mod_list_is_read() {
        let json = r#"{
            "version": {"name": "1.12.2", "protocol": 340},
            "modinfo": {"type": "FML", "modList": [
                {"modid": "minecraft", "version": "1.12.2"},
                {"modid": "jei", "version": "4.16.1.301"}
            ]}
        }"#;
        let s = parse_status(json).unwrap();
        assert_eq!(s.version, "1.12.2");
        assert!(s.advertises_mods);
        assert_eq!(s.mods.len(), 2);
        assert_eq!(s.mods[1].modid, "jei");
        assert_eq!(s.mods[1].version, "4.16.1.301");
    }

    // Newer Forge renamed the field and may omit a version entirely.
    #[test]
    fn fml2_mod_list_is_read() {
        let json = r#"{
            "version": {"name": "1.20.1"},
            "forgeData": {"mods": [
                {"modId": "forge", "version": "47.2.0"},
                {"modId": "jei"}
            ]}
        }"#;
        let s = parse_status(json).unwrap();
        assert!(s.advertises_mods);
        assert_eq!(s.mods[1].modid, "jei");
        assert_eq!(s.mods[1].version, "");
    }

    // "No mods" and "will not say" must not read the same: a spoof built from
    // silence would be a guess wearing the shape of an answer.
    #[test]
    fn a_server_that_says_nothing_is_not_a_server_with_no_mods() {
        let vanilla = parse_status(r#"{"version": {"name": "1.20.4"}}"#).unwrap();
        assert!(!vanilla.advertises_mods);
        assert!(vanilla.mods.is_empty());

        let empty_list =
            parse_status(r#"{"version":{"name":"1.12.2"},"modinfo":{"modList":[]}}"#).unwrap();
        assert!(
            empty_list.advertises_mods,
            "an empty list is an answer; absence is not"
        );
    }
}
