use crate::{Ipv4Address, Seq};
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PackedTuple {
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
    pub src_port: u16,
    pub dst_port: u16,
    pub proto: u8,
    pub _pad: [u8; 3],
}
impl PackedTuple {
    #[inline(always)]
    pub fn new(local_ip: Ipv4Address, local_port: u16, remote_ip: Ipv4Address, remote_port: u16) -> Self {
        Self {
            src_ip: local_ip.0,
            dst_ip: remote_ip.0,
            src_port: local_port,
            dst_port: remote_port,
            proto: 6,
            _pad: [0; 3],
        }
    }
}
pub const MAX_SACK_BLOCKS: usize = 4;
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineSackBlocks {
    pub blocks: [(Seq, Seq); MAX_SACK_BLOCKS],
    pub count: u8,
}
impl Default for InlineSackBlocks {
    fn default() -> Self {
        Self {
            blocks: [(Seq(0), Seq(0)); MAX_SACK_BLOCKS],
            count: 0,
        }
    }
}
impl InlineSackBlocks {
    #[inline(always)]
    pub fn clear(&mut self) {
        self.count = 0;
    }
    #[inline(always)]
    pub fn push(&mut self, start: Seq, end: Seq) -> bool {
        if (self.count as usize) < MAX_SACK_BLOCKS {
            self.blocks[self.count as usize] = (start, end);
            self.count += 1;
            true
        } else {
            false
        }
    }
    #[inline(always)]
    pub fn as_slice(&self) -> &[(Seq, Seq)] {
        &self.blocks[..self.count as usize]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_tuple_layout() {
        assert_eq!(std::mem::size_of::<PackedTuple>(), 16);
        assert_eq!(std::mem::align_of::<PackedTuple>(), 16);
    }
    #[test]
    fn inline_sack_blocks_ops() {
        let mut b = InlineSackBlocks::default();
        assert_eq!(b.as_slice().len(), 0);
        assert!(b.push(Seq(100), Seq(200)));
        assert!(b.push(Seq(300), Seq(400)));
        assert_eq!(b.as_slice().len(), 2);
        assert_eq!(b.as_slice()[0], (Seq(100), Seq(200)));
        assert_eq!(b.as_slice()[1], (Seq(300), Seq(400)));
    }
}
