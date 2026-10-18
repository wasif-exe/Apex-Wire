use crate::conntable::ConnTable;
use crate::{Ipv4Address, MacAddress, SocketTuple, TcpConnection};
use std::collections::VecDeque;
pub struct StackShard {
    pub shard_id: usize,
    pub mac: MacAddress,
    pub ip: Ipv4Address,
    pub gateway_ip: Ipv4Address,
    pub connections: ConnTable<SocketTuple, TcpConnection>,
    pub tx_queue: VecDeque<Vec<u8>>,
}
impl StackShard {
    pub fn new(shard_id: usize, mac: MacAddress, ip: Ipv4Address, gateway_ip: Ipv4Address, max_conns: usize) -> Self {
        Self {
            shard_id,
            mac,
            ip,
            gateway_ip,
            connections: ConnTable::new(max_conns),
            tx_queue: VecDeque::new(),
        }
    }
    #[inline(always)]
    pub fn handles_flow(&self, tuple: &SocketTuple, num_shards: usize) -> bool {
        let mut hash = 17u32;
        for &b in tuple.local_ip.0.iter().chain(tuple.remote_ip.0.iter()) {
            hash = hash.wrapping_mul(31).wrapping_add(b as u32);
        }
        hash = hash.wrapping_mul(31).wrapping_add(tuple.local_port as u32);
        hash = hash.wrapping_mul(31).wrapping_add(tuple.remote_port as u32);
        (hash as usize % num_shards) == self.shard_id
    }
}
