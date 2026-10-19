use std::collections::HashMap;
use crate::resp::{Command, RespParser};
pub struct ShardKvStore {
    store: HashMap<Vec<u8>, Vec<u8>>,
}
impl Default for ShardKvStore {
    fn default() -> Self {
        Self::new()
    }
}
impl ShardKvStore {
    pub fn new() -> Self {
        Self {
            store: HashMap::with_capacity(16384),
        }
    }
    pub fn execute(&mut self, cmd: &Command<'_>, out: &mut Vec<u8>) {
        match cmd {
            Command::Ping => {
                RespParser::encode_pong(out);
            }
            Command::Get(key) => {
                let val = self.store.get(*key).map(|v| v.as_slice());
                RespParser::encode_bulk_string(out, val);
            }
            Command::Set(key, val) => {
                self.store.insert(key.to_vec(), val.to_vec());
                RespParser::encode_ok(out);
            }
            Command::Del(key) => {
                let count = if self.store.remove(*key).is_some() { 1 } else { 0 };
                RespParser::encode_integer(out, count);
            }
            Command::Exists(key) => {
                let count = if self.store.contains_key(*key) { 1 } else { 0 };
                RespParser::encode_integer(out, count);
            }
            Command::Unknown => {
                RespParser::encode_error(out, "unknown command");
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kv_store_lifecycle() {
        let mut kv = ShardKvStore::new();
        let mut out = Vec::new();
        kv.execute(&Command::Set(b"name", b"wire"), &mut out);
        assert_eq!(out, b"+OK\r\n");
        out.clear();
        kv.execute(&Command::Get(b"name"), &mut out);
        assert_eq!(out, b"$4\r\nwire\r\n");
        out.clear();
        kv.execute(&Command::Exists(b"name"), &mut out);
        assert_eq!(out, b":1\r\n");
        out.clear();
        kv.execute(&Command::Del(b"name"), &mut out);
        assert_eq!(out, b":1\r\n");
        out.clear();
        kv.execute(&Command::Get(b"name"), &mut out);
        assert_eq!(out, b"$-1\r\n");
    }
}

