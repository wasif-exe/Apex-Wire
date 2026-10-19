use std::collections::HashMap;
pub const DEFAULT_MAX_CONNS: usize = 65536;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnHandle {
    pub index: u32,
    pub generation: u32,
}
impl ConnHandle {
    pub const INVALID: ConnHandle = ConnHandle { index: u32::MAX, generation: 0 };
    #[inline(always)]
    pub fn is_valid(&self) -> bool {
        self.index != u32::MAX
    }
}
pub struct ConnSlot<T> {
    pub generation: u32,
    pub occupied: bool,
    pub data: Option<T>,
}
impl<T> Default for ConnSlot<T> {
    fn default() -> Self {
        Self {
            generation: 0,
            occupied: false,
            data: None,
        }
    }
}
pub struct ConnTable<K: std::hash::Hash + Eq + Clone, T> {
    slots: Vec<ConnSlot<T>>,
    freelist: Vec<u32>,
    index: HashMap<K, u32>,
    capacity: usize,
}
impl<K: std::hash::Hash + Eq + Clone, T> ConnTable<K, T> {
    pub fn new(capacity: usize) -> Self {
        let mut slots = Vec::with_capacity(capacity);
        let mut freelist = Vec::with_capacity(capacity);
        for i in 0..capacity {
            slots.push(ConnSlot::default());
            freelist.push((capacity - 1 - i) as u32);
        }
        Self {
            slots,
            freelist,
            index: HashMap::with_capacity(capacity),
            capacity,
        }
    }
    #[inline]
    pub fn insert(&mut self, key: K, value: T) -> Option<ConnHandle> {
        if self.index.contains_key(&key) {
            return None;
        }
        let idx = self.freelist.pop()?;
        let slot = &mut self.slots[idx as usize];
        slot.generation = slot.generation.wrapping_add(1);
        slot.occupied = true;
        slot.data = Some(value);
        let handle = ConnHandle { index: idx, generation: slot.generation };
        self.index.insert(key, idx);
        Some(handle)
    }
    #[inline]
    pub fn get_by_key(&self, key: &K) -> Option<&T> {
        let idx = *self.index.get(key)?;
        let slot = &self.slots[idx as usize];
        if slot.occupied { slot.data.as_ref() } else { None }
    }
    #[inline]
    pub fn get_mut_by_key(&mut self, key: &K) -> Option<&mut T> {
        let idx = *self.index.get(key)?;
        let slot = &mut self.slots[idx as usize];
        if slot.occupied { slot.data.as_mut() } else { None }
    }
    #[inline]
    pub fn get_by_handle(&self, h: ConnHandle) -> Option<&T> {
        let slot = self.slots.get(h.index as usize)?;
        if slot.occupied && slot.generation == h.generation {
            slot.data.as_ref()
        } else {
            None
        }
    }
    #[inline]
    pub fn get_mut_by_handle(&mut self, h: ConnHandle) -> Option<&mut T> {
        let slot = self.slots.get_mut(h.index as usize)?;
        if slot.occupied && slot.generation == h.generation {
            slot.data.as_mut()
        } else {
            None
        }
    }
    pub fn remove_by_key(&mut self, key: &K) -> Option<T> {
        let idx = self.index.remove(key)?;
        let slot = &mut self.slots[idx as usize];
        slot.occupied = false;
        self.freelist.push(idx);
        slot.data.take()
    }
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.index.keys()
    }
    pub fn len(&self) -> usize {
        self.index.len()
    }
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn iter_mut_values(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut()
            .filter(|s| s.occupied)
            .filter_map(|s| s.data.as_mut())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic_insert_get() {
        let mut t: ConnTable<u32, String> = ConnTable::new(16);
        let h = t.insert(42, "hello".to_string()).unwrap();
        assert_eq!(t.get_by_key(&42).unwrap(), "hello");
        assert_eq!(t.get_by_handle(h).unwrap(), "hello");
    }
    #[test]
    fn remove_and_reuse() {
        let mut t: ConnTable<u32, String> = ConnTable::new(16);
        let _h1 = t.insert(1, "a".to_string()).unwrap();
        t.remove_by_key(&1);
        assert!(t.get_by_key(&1).is_none());
        let h2 = t.insert(1, "b".to_string()).unwrap();
        assert_eq!(t.get_by_handle(h2).unwrap(), "b");
    }
    #[test]
    fn generation_prevents_aba() {
        let mut t: ConnTable<u32, String> = ConnTable::new(16);
        let h_old = t.insert(1, "a".to_string()).unwrap();
        t.remove_by_key(&1);
        let _h_new = t.insert(1, "b".to_string()).unwrap();
        assert!(t.get_by_handle(h_old).is_none());
    }
    #[test]
    fn capacity_limit() {
        let mut t: ConnTable<u32, u32> = ConnTable::new(4);
        for i in 0..4 { t.insert(i, i).unwrap(); }
        assert!(t.insert(99, 99).is_none());
    }
}

