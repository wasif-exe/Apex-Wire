#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[repr(C, align(32))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackedOrderSlots {
    pub prices: [u32; 8],
    pub shares: [u32; 8],
    pub user_ids: [u32; 8],
    pub flags: [u32; 8],
}

impl Default for PackedOrderSlots {
    #[inline(always)]
    fn default() -> Self {
        Self {
            prices: [0; 8],
            shares: [0; 8],
            user_ids: [0; 8],
            flags: [0; 8],
        }
    }
}

pub const FLAG_ACTIVE: u32 = 1 << 0;
pub const FLAG_CANCELLED: u32 = 1 << 1;

#[inline(always)]
pub fn batch_cancel_by_user_x8(slots: &mut PackedOrderSlots, target_user_id: u32) -> u8 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Direct AVX2 execution on 32-byte aligned structures.
        return unsafe { batch_cancel_by_user_avx2(slots, target_user_id) };
    }
    #[allow(unreachable_code)]
    batch_cancel_by_user_scalar(slots, target_user_id)
}

#[inline(always)]
pub fn batch_cancel_by_price_below_x8(slots: &mut PackedOrderSlots, min_price: u32) -> u8 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Direct AVX2 execution on 32-byte aligned structures.
        return unsafe { batch_cancel_by_price_below_avx2(slots, min_price) };
    }
    #[allow(unreachable_code)]
    batch_cancel_by_price_below_scalar(slots, min_price)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
pub unsafe fn batch_cancel_by_user_avx2(slots: &mut PackedOrderSlots, target_user_id: u32) -> u8 {
    let target_vec = _mm256_set1_epi32(target_user_id as i32);
    let users_vec = _mm256_load_si256(slots.user_ids.as_ptr() as *const __m256i);
    let cmp = _mm256_cmpeq_epi32(users_vec, target_vec);

    let active_mask_vec = _mm256_set1_epi32(FLAG_ACTIVE as i32);
    let flags_vec = _mm256_load_si256(slots.flags.as_ptr() as *const __m256i);
    let is_active = _mm256_and_si256(flags_vec, active_mask_vec);
    let active_cmp = _mm256_cmpeq_epi32(is_active, active_mask_vec);

    let match_vec = _mm256_and_si256(cmp, active_cmp);
    _mm256_movemask_ps(_mm256_castsi256_ps(match_vec)) as u8
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
pub unsafe fn batch_cancel_by_price_below_avx2(slots: &mut PackedOrderSlots, min_price: u32) -> u8 {
    let min_vec = _mm256_set1_epi32(min_price as i32);
    let prices_vec = _mm256_load_si256(slots.prices.as_ptr() as *const __m256i);
    let lt_cmp = _mm256_cmpgt_epi32(min_vec, prices_vec);

    let active_mask_vec = _mm256_set1_epi32(FLAG_ACTIVE as i32);
    let flags_vec = _mm256_load_si256(slots.flags.as_ptr() as *const __m256i);
    let is_active = _mm256_and_si256(flags_vec, active_mask_vec);
    let active_cmp = _mm256_cmpeq_epi32(is_active, active_mask_vec);

    let match_vec = _mm256_and_si256(lt_cmp, active_cmp);
    _mm256_movemask_ps(_mm256_castsi256_ps(match_vec)) as u8
}

#[inline(always)]
pub fn batch_cancel_by_user_scalar(slots: &mut PackedOrderSlots, target_user_id: u32) -> u8 {
    let mut mask = 0u8;
    for i in 0..8 {
        if slots.user_ids[i] == target_user_id && (slots.flags[i] & FLAG_ACTIVE) != 0 {
            mask |= 1 << i;
        }
    }
    mask
}

#[inline(always)]
pub fn batch_cancel_by_price_below_scalar(slots: &mut PackedOrderSlots, min_price: u32) -> u8 {
    let mut mask = 0u8;
    for i in 0..8 {
        if slots.prices[i] < min_price && (slots.flags[i] & FLAG_ACTIVE) != 0 {
            mask |= 1 << i;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_cancel_user_match() {
        let mut slots = PackedOrderSlots::default();
        slots.user_ids = [10, 20, 10, 30, 10, 40, 50, 10];
        slots.flags = [FLAG_ACTIVE; 8];
        slots.prices = [18000; 8];
        slots.shares = [100; 8];

        let mask = batch_cancel_by_user_x8(&mut slots, 10);
        assert_eq!(mask, 0b10010101);
    }

    #[test]
    fn batch_cancel_price_below() {
        let mut slots = PackedOrderSlots::default();
        slots.prices = [17990, 18005, 17980, 18010, 17950, 18000, 18020, 17900];
        slots.flags = [FLAG_ACTIVE; 8];
        slots.user_ids = [1; 8];
        slots.shares = [100; 8];

        let mask = batch_cancel_by_price_below_x8(&mut slots, 18000);
        assert_eq!(mask, 0b10010101);
    }

    #[test]
    fn packed_slots_alignment() {
        assert_eq!(std::mem::align_of::<PackedOrderSlots>(), 32);
        assert_eq!(std::mem::size_of::<PackedOrderSlots>(), 128);
    }
}
