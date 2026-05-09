use js_sys::Int32Array;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

const X_MULT: u32 = 3_129_871;
const Z_MULT_VANILLA: u32 = 116_129_781;
const Z_MULT_TB3: u32 = 6_129_781;
const LCG_MULT: u32 = 42_317_861;
const LCG_ADDEND: u32 = 11;
const MIX_INPUT_MASK: u32 = (1 << 28) - 1;
const MIX_OUTPUT_MASK: u32 = 0x0fff;
const MIX_LOW_BITS: u32 = 16;
const MIX_HIGH_BITS: u32 = 12;
const MIX_LOW_COUNT: u32 = 1 << MIX_LOW_BITS;
const MIX_HIGH_MASK: u32 = (1 << MIX_HIGH_BITS) - 1;
const PREFILTER_MAX_Y_COUNT: i32 = 64;
const PREFILTER_FINGERPRINT_BITS: usize = 2048;
const PREFILTER_FINGERPRINT_WORDS: usize = PREFILTER_FINGERPRINT_BITS / 64;
const PREFILTER_FINGERPRINT_MASK: u32 = (PREFILTER_FINGERPRINT_BITS as u32) - 1;

const SEED_POST_1_8: u8 = 0;
const SEED_PRE_1_8: u8 = 1;
const SEED_B1_6_TB3: u8 = 2;

const STRICT_FULL_MASK: u8 = 0;
const STRICT_SIMPLE_MASK: u8 = 1;
const STRICT_MIXED: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SeedMode {
    /// 1.8+ block-model random offset: X/Z only, same offset at every Y.
    Post1_8,
    /// 1.7.10-style foliage random offset: X/Y/Z all affect the hash.
    Pre1_8,
    /// b1.6-tb3 RenderBlocks seed: x * 3129871 + z * 6129781 + y.
    Beta16Tb3,
}

impl SeedMode {
    #[inline(always)]
    fn from_code(code: u8) -> Result<Self, JsValue> {
        match code {
            SEED_POST_1_8 => Ok(Self::Post1_8),
            SEED_PRE_1_8 => Ok(Self::Pre1_8),
            SEED_B1_6_TB3 => Ok(Self::Beta16Tb3),
            _ => Err(JsValue::from_str("Unknown seed mode.")),
        }
    }

    #[inline(always)]
    fn from_legacy_post_flag(post1_12_any_y: bool) -> Self {
        if post1_12_any_y {
            Self::Post1_8
        } else {
            Self::Pre1_8
        }
    }

    #[cfg(test)]
    #[inline(always)]
    fn ignores_y(self) -> bool {
        matches!(self, Self::Post1_8)
    }

    #[inline(always)]
    fn z_multiplier(self) -> u32 {
        match self {
            Self::Beta16Tb3 => Z_MULT_TB3,
            Self::Post1_8 | Self::Pre1_8 => Z_MULT_VANILLA,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    dx: i32,
    dy: i32,
    dz: i32,
    dx_seed: u32,
    dz_seed: u32,
    expected: u16,
    mask: u16,
    dripstone: bool,
}

#[derive(Clone, Copy, Debug)]
struct BaseYMask {
    base: u32,
    y_mask: u64,
}

#[derive(Clone, Copy, Debug)]
struct LowHighYMask {
    high: u16,
    y_mask: u64,
}

#[derive(Debug)]
struct StrictYCache {
    y0: i32,
    y1: i32,
    mode: SeedMode,
    kind: u8,
    pivot_index: usize,
    pivot: Sample,
    fingerprints: Vec<u64>,
    low_offsets: Vec<u32>,
    low_entries: Vec<LowHighYMask>,
}

impl Sample {
    #[inline(always)]
    fn new(
        dx: i32,
        dy: i32,
        dz: i32,
        expected: u16,
        mask: u16,
        dripstone: bool,
        mode: SeedMode,
    ) -> Self {
        let mask = mask & 0x0fff;
        Self {
            dx,
            dy,
            dz,
            dx_seed: (dx as u32).wrapping_mul(X_MULT),
            dz_seed: (dz as u32).wrapping_mul(mode.z_multiplier()),
            expected: expected & mask,
            mask,
            dripstone,
        }
    }

    #[inline(always)]
    fn constraint_bits(self) -> u32 {
        self.mask.count_ones()
    }

    #[inline(always)]
    fn distance_rank(self) -> u32 {
        self.dx.unsigned_abs() + self.dy.unsigned_abs() + self.dz.unsigned_abs()
    }
}

fn validate_lengths(
    rel_dx: &[i32],
    rel_dy: &[i32],
    rel_dz: &[i32],
    rel_packed: &[u16],
    rel_mask: &[u16],
    rel_drip: &[u8],
) -> Result<usize, JsValue> {
    let n = rel_dx.len();
    if rel_dy.len() != n
        || rel_dz.len() != n
        || rel_packed.len() != n
        || rel_mask.len() != n
        || rel_drip.len() != n
    {
        return Err(JsValue::from_str("Input arrays must have the same length."));
    }
    Ok(n)
}

#[inline(always)]
fn validate_bounds(x0: i32, x1: i32, y0: i32, y1: i32, z0: i32, z1: i32) -> Result<(), JsValue> {
    if x0 > x1 || y0 > y1 || z0 > z1 {
        Err(JsValue::from_str("Invalid bounds (min > max)."))
    } else {
        Ok(())
    }
}

fn make_samples(
    rel_dx: &[i32],
    rel_dy: &[i32],
    rel_dz: &[i32],
    rel_packed: &[u16],
    rel_mask: &[u16],
    rel_drip: &[u8],
    mode: SeedMode,
) -> Result<Vec<Sample>, JsValue> {
    let n = validate_lengths(rel_dx, rel_dy, rel_dz, rel_packed, rel_mask, rel_drip)?;
    let mut samples = Vec::with_capacity(n);

    for i in 0..n {
        samples.push(Sample::new(
            rel_dx[i],
            rel_dy[i],
            rel_dz[i],
            rel_packed[i],
            rel_mask[i],
            rel_drip[i] != 0,
            mode,
        ));
    }

    // Fail fast: exact, high-information samples reject candidates most cheaply.
    samples.sort_by(|a, b| {
        a.dripstone
            .cmp(&b.dripstone)
            .then_with(|| b.constraint_bits().cmp(&a.constraint_bits()))
            .then_with(|| b.distance_rank().cmp(&a.distance_rank()))
    });

    Ok(samples)
}

/// The offset nibbles are bits 16..27. Those bits depend only on the low 32 bits
/// of Minecraft's long hash, so wasm can avoid slower i64 arithmetic here.
#[inline(always)]
fn mix_low_12(seed_low: u32) -> u16 {
    let mixed = seed_low
        .wrapping_mul(seed_low)
        .wrapping_mul(LCG_MULT)
        .wrapping_add(seed_low.wrapping_mul(LCG_ADDEND));
    ((mixed >> 16) & 0x0fff) as u16
}

#[inline(always)]
fn inv_odd_mod_4096(a: u32) -> u32 {
    debug_assert_eq!(a & 1, 1);
    let mut x = 1u32;
    for _ in 0..4 {
        x = x.wrapping_mul(2u32.wrapping_sub(a.wrapping_mul(x))) & MIX_HIGH_MASK;
    }
    x
}

/// Invert `mix_low_12` for one exact 12-bit output.
///
/// For `s = low16 + (high12 << 16)`, bits 16..27 of
/// `LCG_MULT*s*s + LCG_ADDEND*s` are linear in `high12` once `low16`
/// is fixed:
///
/// `out = base(low16) + high12 * (LCG_ADDEND + 2*LCG_MULT*low16) (mod 4096)`.
///
/// The coefficient is always odd, so every low16 has exactly one high12.
fn invert_exact_mix_low_12(target: u16) -> Vec<u32> {
    let target = (target as u32) & MIX_OUTPUT_MASK;
    let mut out = Vec::with_capacity(MIX_LOW_COUNT as usize);

    for low in 0..MIX_LOW_COUNT {
        let low64 = low as u64;
        let base = (((low64 * low64 * (LCG_MULT as u64) + low64 * (LCG_ADDEND as u64))
            >> MIX_LOW_BITS)
            & (MIX_OUTPUT_MASK as u64)) as u32;
        let coeff =
            (LCG_ADDEND.wrapping_add(2u32.wrapping_mul(LCG_MULT & MIX_OUTPUT_MASK).wrapping_mul(low)))
                & MIX_OUTPUT_MASK;
        let high = target
            .wrapping_sub(base)
            .wrapping_mul(inv_odd_mod_4096(coeff))
            & MIX_HIGH_MASK;
        out.push(low | (high << MIX_LOW_BITS));
    }

    out
}

#[cfg(test)]
#[inline(always)]
fn seed_from_parts(mode: SeedMode, sample: Sample, x_seed: u32, y: i32, z_seed: u32) -> u32 {
    let x_part = x_seed.wrapping_add(sample.dx_seed);
    let z_part = z_seed.wrapping_add(sample.dz_seed);
    let y_part = if mode.ignores_y() {
        0
    } else {
        y.wrapping_add(sample.dy) as u32
    };

    match mode {
        SeedMode::Beta16Tb3 => x_part.wrapping_add(z_part).wrapping_add(y_part),
        SeedMode::Post1_8 | SeedMode::Pre1_8 => x_part ^ z_part ^ y_part,
    }
}

#[cfg(test)]
#[inline(always)]
fn packed_from_parts(mode: SeedMode, sample: Sample, x_seed: u32, y: i32, z_seed: u32) -> u16 {
    mix_low_12(seed_from_parts(mode, sample, x_seed, y, z_seed))
}

#[inline(always)]
fn z_multiplier_for_code<const MODE: u8>() -> u32 {
    if MODE == SEED_B1_6_TB3 {
        Z_MULT_TB3
    } else {
        Z_MULT_VANILLA
    }
}

#[inline(always)]
fn seed_from_parts_for_code<const MODE: u8>(
    sample: Sample,
    x_seed: u32,
    y: i32,
    z_seed: u32,
) -> u32 {
    let x_part = x_seed.wrapping_add(sample.dx_seed);
    let z_part = z_seed.wrapping_add(sample.dz_seed);

    if MODE == SEED_POST_1_8 {
        x_part ^ z_part
    } else {
        let y_part = y.wrapping_add(sample.dy) as u32;
        if MODE == SEED_B1_6_TB3 {
            x_part.wrapping_add(z_part).wrapping_add(y_part)
        } else {
            x_part ^ z_part ^ y_part
        }
    }
}

#[inline(always)]
fn packed_from_parts_for_code<const MODE: u8>(
    sample: Sample,
    x_seed: u32,
    y: i32,
    z_seed: u32,
) -> u16 {
    mix_low_12(seed_from_parts_for_code::<MODE>(sample, x_seed, y, z_seed))
}

#[inline(always)]
fn axis_nibble(v: u16, axis: u32) -> u8 {
    ((v >> (axis * 4)) & 0x000f) as u8
}

#[inline(always)]
fn dripstone_nibble_matches(expected: u8, predicted: u8) -> bool {
    if expected <= 3 {
        predicted <= 3
    } else if expected >= 12 {
        predicted >= 12
    } else {
        predicted == expected
    }
}

#[inline(always)]
fn dripstone_nibble_distance(expected: u8, predicted: u8) -> i32 {
    if expected <= 3 {
        if predicted <= 3 {
            0
        } else {
            (predicted - 3) as i32
        }
    } else if expected >= 12 {
        if predicted >= 12 {
            0
        } else {
            (12 - predicted) as i32
        }
    } else {
        (predicted as i32 - expected as i32).abs()
    }
}

#[inline(always)]
fn strict_sample_matches(sample: Sample, pred: u16) -> bool {
    if !sample.dripstone {
        return (pred & sample.mask) == sample.expected;
    }

    for axis in 0..3 {
        let nib_mask = (sample.mask >> (axis * 4)) & 0x000f;
        if nib_mask == 0 {
            continue;
        }

        let predicted = axis_nibble(pred, axis);
        let expected = axis_nibble(sample.expected, axis);
        if axis == 1 {
            if predicted != expected {
                return false;
            }
        } else if !dripstone_nibble_matches(expected, predicted) {
            return false;
        }
    }

    true
}

#[inline(always)]
fn add_sample_score(sample: Sample, pred: u16, tol: i32, score: &mut i32, max_score: i32) -> bool {
    for axis in 0..3 {
        let nib_mask = (sample.mask >> (axis * 4)) & 0x000f;
        if nib_mask == 0 {
            continue;
        }

        let predicted = axis_nibble(pred, axis);
        let expected = axis_nibble(sample.expected, axis);
        let d = if sample.dripstone && axis != 1 {
            dripstone_nibble_distance(expected, predicted)
        } else {
            (predicted as i32 - expected as i32).abs()
        };

        *score += if d <= tol { d } else { d * d };
        if *score > max_score {
            return false;
        }
    }

    true
}

fn strict_check_kind(samples: &[Sample]) -> u8 {
    let mut all_simple = true;
    let mut all_full_mask = true;

    for &sample in samples {
        if sample.dripstone {
            all_simple = false;
            all_full_mask = false;
            break;
        }
        if sample.mask != 0x0fff {
            all_full_mask = false;
        }
    }

    if all_full_mask {
        STRICT_FULL_MASK
    } else if all_simple {
        STRICT_SIMPLE_MASK
    } else {
        STRICT_MIXED
    }
}

fn choose_strict_y_pivot(samples: &[Sample]) -> Option<usize> {
    samples
        .iter()
        .position(|sample| !sample.dripstone && sample.mask == 0x0fff)
}

fn build_strict_y_cache(
    mode: SeedMode,
    kind: u8,
    samples: &[Sample],
    y0: i32,
    y1: i32,
) -> Option<StrictYCache> {
    if !matches!(mode, SeedMode::Pre1_8) || y0 > y1 {
        return None;
    }

    let y_count = y1.wrapping_sub(y0).wrapping_add(1);
    if !(1..=PREFILTER_MAX_Y_COUNT).contains(&y_count) {
        return None;
    }

    let pivot_index = choose_strict_y_pivot(samples)?;
    let pivot = samples[pivot_index];
    let residues = invert_exact_mix_low_12(pivot.expected);
    let mut fingerprints = vec![0u64; (1usize << 16) * PREFILTER_FINGERPRINT_WORDS];
    let mut entries =
        Vec::with_capacity(residues.len().saturating_mul(y_count as usize).min(8_388_608));

    for y_offset in 0..(y_count as u32) {
        let y = y0.wrapping_add(y_offset as i32);
        let y_part = y.wrapping_add(pivot.dy) as u32;
        let y_bit = 1u64 << y_offset;

        for &seed_low28 in &residues {
            let base = match mode {
                SeedMode::Pre1_8 => seed_low28 ^ (y_part & MIX_INPUT_MASK),
                SeedMode::Beta16Tb3 => seed_low28.wrapping_sub(y_part) & MIX_INPUT_MASK,
                SeedMode::Post1_8 => unreachable!(),
            };

            let low = (base & 0xffff) as usize;
            let bucket = (base >> 16) & PREFILTER_FINGERPRINT_MASK;
            fingerprints[low * PREFILTER_FINGERPRINT_WORDS + ((bucket >> 6) as usize)] |=
                1u64 << (bucket & 63);
            entries.push(BaseYMask {
                base,
                y_mask: y_bit,
            });
        }
    }

    entries.sort_unstable_by(|a, b| {
        (a.base & 0xffff)
            .cmp(&(b.base & 0xffff))
            .then_with(|| (a.base >> 16).cmp(&(b.base >> 16)))
            .then_with(|| a.y_mask.cmp(&b.y_mask))
    });

    let mut low_offsets = vec![0u32; (1usize << 16) + 1];
    let mut low_entries: Vec<LowHighYMask> = Vec::with_capacity(entries.len());
    let mut next_offset_low = 0usize;
    let mut last_low: Option<usize> = None;

    for entry in entries {
        let low = (entry.base & 0xffff) as usize;
        let high = ((entry.base >> 16) & MIX_HIGH_MASK) as u16;

        while next_offset_low <= low {
            low_offsets[next_offset_low] = low_entries.len() as u32;
            next_offset_low += 1;
        }

        if last_low == Some(low) {
            if let Some(last) = low_entries.last_mut() {
                if last.high == high {
                    last.y_mask |= entry.y_mask;
                    continue;
                }
            }
        }

        low_entries.push(LowHighYMask {
            high,
            y_mask: entry.y_mask,
        });
        last_low = Some(low);
    }

    while next_offset_low < low_offsets.len() {
        low_offsets[next_offset_low] = low_entries.len() as u32;
        next_offset_low += 1;
    }

    Some(StrictYCache {
        y0,
        y1,
        mode,
        kind,
        pivot_index,
        pivot,
        fingerprints,
        low_offsets,
        low_entries,
    })
}

#[inline(always)]
fn strict_y_cache_matches(cache: &StrictYCache, mode: SeedMode, kind: u8, y0: i32, y1: i32) -> bool {
    cache.mode == mode && cache.kind == kind && cache.y0 == y0 && cache.y1 == y1
}

#[inline(always)]
fn strict_y_cache_y_mask(cache: &StrictYCache, base: u32) -> u64 {
    let low = (base & 0xffff) as usize;
    let high = (base >> 16) as u16;
    let bucket = (high as u32) & PREFILTER_FINGERPRINT_MASK;
    let fingerprint_index = low * PREFILTER_FINGERPRINT_WORDS + ((bucket >> 6) as usize);

    if (cache.fingerprints[fingerprint_index] & (1u64 << (bucket & 63))) == 0 {
        return 0;
    }

    let start = cache.low_offsets[low] as usize;
    let end = cache.low_offsets[low + 1] as usize;
    for entry in &cache.low_entries[start..end] {
        if entry.high == high {
            return entry.y_mask;
        }
    }

    0
}

#[inline(always)]
fn candidate_strict_for_code<const MODE: u8, const KIND: u8>(
    samples: &[Sample],
    x_seed: u32,
    y: i32,
    z_seed: u32,
) -> bool {
    for &sample in samples {
        let pred = packed_from_parts_for_code::<MODE>(sample, x_seed, y, z_seed);
        if KIND == STRICT_FULL_MASK {
            if pred != sample.expected {
                return false;
            }
        } else if KIND == STRICT_SIMPLE_MASK {
            if (pred & sample.mask) != sample.expected {
                return false;
            }
        } else if !strict_sample_matches(sample, pred) {
            return false;
        }
    }

    true
}

#[inline(always)]
fn candidate_strict_for_code_skip<const MODE: u8, const KIND: u8>(
    samples: &[Sample],
    skip_index: usize,
    x_seed: u32,
    y: i32,
    z_seed: u32,
) -> bool {
    for (index, &sample) in samples.iter().enumerate() {
        if index == skip_index {
            continue;
        }

        let pred = packed_from_parts_for_code::<MODE>(sample, x_seed, y, z_seed);
        if KIND == STRICT_FULL_MASK {
            if pred != sample.expected {
                return false;
            }
        } else if KIND == STRICT_SIMPLE_MASK {
            if (pred & sample.mask) != sample.expected {
                return false;
            }
        } else if !strict_sample_matches(sample, pred) {
            return false;
        }
    }

    true
}

#[inline(always)]
fn pivot_base_for_code<const MODE: u8>(pivot: Sample, x_seed: u32, z_seed: u32) -> u32 {
    let x_part = x_seed.wrapping_add(pivot.dx_seed);
    let z_part = z_seed.wrapping_add(pivot.dz_seed);

    if MODE == SEED_B1_6_TB3 {
        x_part.wrapping_add(z_part) & MIX_INPUT_MASK
    } else {
        (x_part ^ z_part) & MIX_INPUT_MASK
    }
}

#[inline(always)]
fn candidate_score_for_code<const MODE: u8>(
    samples: &[Sample],
    x_seed: u32,
    y: i32,
    z_seed: u32,
    tol: i32,
    max_score: i32,
) -> Option<i32> {
    let mut score = 0;

    for &sample in samples {
        let pred = packed_from_parts_for_code::<MODE>(sample, x_seed, y, z_seed);
        if !add_sample_score(sample, pred, tol, &mut score, max_score) {
            return None;
        }
    }

    Some(score)
}

fn scan_strict_loop<const MODE: u8, const KIND: u8>(
    samples: &[Sample],
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
) -> Result<Int32Array, JsValue> {
    validate_bounds(x0, x1, y0, y1, z0, z1)?;
    if max_matches == 0 {
        return Ok(Int32Array::new_with_length(0));
    }

    let z_multiplier = z_multiplier_for_code::<MODE>();
    let mut out: Vec<i32> = Vec::with_capacity((max_matches as usize).saturating_mul(3).min(4096));

    let y_start = y0;
    let y_end = if MODE == SEED_POST_1_8 { y0 } else { y1 };

    let mut y = y_start;
    loop {
        let mut z = z0;
        let mut z_seed = (z as u32).wrapping_mul(z_multiplier);
        loop {
            let mut x = x0;
            let mut x_seed = (x as u32).wrapping_mul(X_MULT);
            loop {
                if candidate_strict_for_code::<MODE, KIND>(samples, x_seed, y, z_seed) {
                    out.push(x);
                    out.push(y);
                    out.push(z);
                    if (out.len() / 3) as u32 >= max_matches {
                        return Ok(Int32Array::from(out.as_slice()));
                    }
                }

                if x == x1 {
                    break;
                }
                x = x.wrapping_add(1);
                x_seed = x_seed.wrapping_add(X_MULT);
            }

            if z == z1 {
                break;
            }
            z = z.wrapping_add(1);
            z_seed = z_seed.wrapping_add(z_multiplier);
        }

        if y == y_end {
            break;
        }
        y = y.wrapping_add(1);
    }

    Ok(Int32Array::from(out.as_slice()))
}

fn scan_strict_loop_y_prefilter<const MODE: u8, const KIND: u8>(
    samples: &[Sample],
    cache: &StrictYCache,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
) -> Result<Int32Array, JsValue> {
    validate_bounds(x0, x1, y0, y1, z0, z1)?;
    if max_matches == 0 {
        return Ok(Int32Array::new_with_length(0));
    }

    debug_assert_eq!(cache.y0, y0);
    debug_assert_eq!(cache.y1, y1);

    let z_multiplier = z_multiplier_for_code::<MODE>();
    let mut out: Vec<i32> = Vec::with_capacity((max_matches as usize).saturating_mul(3).min(4096));

    let mut z = z0;
    let mut z_seed = (z as u32).wrapping_mul(z_multiplier);
    loop {
        let mut x = x0;
        let mut x_seed = (x as u32).wrapping_mul(X_MULT);
        loop {
            let base = pivot_base_for_code::<MODE>(cache.pivot, x_seed, z_seed);
            let mut y_mask = strict_y_cache_y_mask(cache, base);
            while y_mask != 0 {
                let y_offset = y_mask.trailing_zeros() as i32;
                y_mask &= y_mask - 1;
                let y = y0.wrapping_add(y_offset);

                if candidate_strict_for_code_skip::<MODE, KIND>(
                    samples,
                    cache.pivot_index,
                    x_seed,
                    y,
                    z_seed,
                ) {
                    out.push(x);
                    out.push(y);
                    out.push(z);
                    if (out.len() / 3) as u32 >= max_matches {
                        return Ok(Int32Array::from(out.as_slice()));
                    }
                }
            }

            if x == x1 {
                break;
            }
            x = x.wrapping_add(1);
            x_seed = x_seed.wrapping_add(X_MULT);
        }

        if z == z1 {
            break;
        }
        z = z.wrapping_add(1);
        z_seed = z_seed.wrapping_add(z_multiplier);
    }

    Ok(Int32Array::from(out.as_slice()))
}

fn scan_strict_prepared_with_y_cache(
    mode: SeedMode,
    samples: &[Sample],
    cache: &StrictYCache,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
) -> Result<Int32Array, JsValue> {
    match mode {
        SeedMode::Pre1_8 => match cache.kind {
            STRICT_FULL_MASK => scan_strict_loop_y_prefilter::<SEED_PRE_1_8, STRICT_FULL_MASK>(
                samples,
                cache,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            STRICT_SIMPLE_MASK => scan_strict_loop_y_prefilter::<SEED_PRE_1_8, STRICT_SIMPLE_MASK>(
                samples,
                cache,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            _ => scan_strict_loop_y_prefilter::<SEED_PRE_1_8, STRICT_MIXED>(
                samples,
                cache,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
        },
        SeedMode::Beta16Tb3 => match cache.kind {
            STRICT_FULL_MASK => scan_strict_loop_y_prefilter::<SEED_B1_6_TB3, STRICT_FULL_MASK>(
                samples,
                cache,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            STRICT_SIMPLE_MASK => scan_strict_loop_y_prefilter::<SEED_B1_6_TB3, STRICT_SIMPLE_MASK>(
                samples,
                cache,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            _ => scan_strict_loop_y_prefilter::<SEED_B1_6_TB3, STRICT_MIXED>(
                samples,
                cache,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
        },
        SeedMode::Post1_8 => scan_strict_prepared(
            mode,
            samples,
            x0,
            x1,
            y0,
            y1,
            z0,
            z1,
            max_matches,
        ),
    }
}

fn scan_strict_prepared(
    mode: SeedMode,
    samples: &[Sample],
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
) -> Result<Int32Array, JsValue> {
    let kind = strict_check_kind(samples);
    match mode {
        SeedMode::Post1_8 => match kind {
            STRICT_FULL_MASK => scan_strict_loop::<SEED_POST_1_8, STRICT_FULL_MASK>(
                samples,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            STRICT_SIMPLE_MASK => scan_strict_loop::<SEED_POST_1_8, STRICT_SIMPLE_MASK>(
                samples,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            _ => scan_strict_loop::<SEED_POST_1_8, STRICT_MIXED>(
                samples,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
        },
        SeedMode::Pre1_8 => match kind {
            STRICT_FULL_MASK => scan_strict_loop::<SEED_PRE_1_8, STRICT_FULL_MASK>(
                samples,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            STRICT_SIMPLE_MASK => scan_strict_loop::<SEED_PRE_1_8, STRICT_SIMPLE_MASK>(
                samples,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            _ => scan_strict_loop::<SEED_PRE_1_8, STRICT_MIXED>(
                samples,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
        },
        SeedMode::Beta16Tb3 => match kind {
            STRICT_FULL_MASK => scan_strict_loop::<SEED_B1_6_TB3, STRICT_FULL_MASK>(
                samples,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            STRICT_SIMPLE_MASK => scan_strict_loop::<SEED_B1_6_TB3, STRICT_SIMPLE_MASK>(
                samples,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
            _ => scan_strict_loop::<SEED_B1_6_TB3, STRICT_MIXED>(
                samples,
                x0,
                x1,
                y0,
                y1,
                z0,
                z1,
                max_matches,
            ),
        },
    }
}

fn scan_strict_impl(
    rel_dx: &[i32],
    rel_dy: &[i32],
    rel_dz: &[i32],
    rel_packed: &[u16],
    rel_mask: &[u16],
    rel_drip: &[u8],
    mode: SeedMode,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
) -> Result<Int32Array, JsValue> {
    validate_bounds(x0, x1, y0, y1, z0, z1)?;
    if max_matches == 0 {
        return Ok(Int32Array::new_with_length(0));
    }

    let samples = make_samples(rel_dx, rel_dy, rel_dz, rel_packed, rel_mask, rel_drip, mode)?;
    let kind = strict_check_kind(&samples);
    if let Some(cache) = build_strict_y_cache(mode, kind, &samples, y0, y1) {
        return scan_strict_prepared_with_y_cache(
            mode,
            &samples,
            &cache,
            x0,
            x1,
            y0,
            y1,
            z0,
            z1,
            max_matches,
        );
    }

    scan_strict_prepared(mode, &samples, x0, x1, y0, y1, z0, z1, max_matches)
}

fn scan_scored_loop<const MODE: u8>(
    samples: &[Sample],
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
    tol: u8,
    max_score: i32,
) -> Result<Int32Array, JsValue> {
    validate_bounds(x0, x1, y0, y1, z0, z1)?;
    if max_matches == 0 {
        return Ok(Int32Array::new_with_length(0));
    }

    let z_multiplier = z_multiplier_for_code::<MODE>();
    let tol = tol as i32;
    let mut out: Vec<i32> = Vec::with_capacity((max_matches as usize).saturating_mul(4).min(4096));

    let y_start = y0;
    let y_end = if MODE == SEED_POST_1_8 { y0 } else { y1 };

    let mut y = y_start;
    loop {
        let mut z = z0;
        let mut z_seed = (z as u32).wrapping_mul(z_multiplier);
        loop {
            let mut x = x0;
            let mut x_seed = (x as u32).wrapping_mul(X_MULT);
            loop {
                if let Some(score) =
                    candidate_score_for_code::<MODE>(samples, x_seed, y, z_seed, tol, max_score)
                {
                    out.push(x);
                    out.push(y);
                    out.push(z);
                    out.push(score);
                    if (out.len() / 4) as u32 >= max_matches {
                        return Ok(Int32Array::from(out.as_slice()));
                    }
                }

                if x == x1 {
                    break;
                }
                x = x.wrapping_add(1);
                x_seed = x_seed.wrapping_add(X_MULT);
            }

            if z == z1 {
                break;
            }
            z = z.wrapping_add(1);
            z_seed = z_seed.wrapping_add(z_multiplier);
        }

        if y == y_end {
            break;
        }
        y = y.wrapping_add(1);
    }

    Ok(Int32Array::from(out.as_slice()))
}

fn scan_scored_prepared(
    mode: SeedMode,
    samples: &[Sample],
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
    tol: u8,
    max_score: i32,
) -> Result<Int32Array, JsValue> {
    match mode {
        SeedMode::Post1_8 => scan_scored_loop::<SEED_POST_1_8>(
            samples,
            x0,
            x1,
            y0,
            y1,
            z0,
            z1,
            max_matches,
            tol,
            max_score,
        ),
        SeedMode::Pre1_8 => scan_scored_loop::<SEED_PRE_1_8>(
            samples,
            x0,
            x1,
            y0,
            y1,
            z0,
            z1,
            max_matches,
            tol,
            max_score,
        ),
        SeedMode::Beta16Tb3 => scan_scored_loop::<SEED_B1_6_TB3>(
            samples,
            x0,
            x1,
            y0,
            y1,
            z0,
            z1,
            max_matches,
            tol,
            max_score,
        ),
    }
}

fn scan_scored_impl(
    rel_dx: &[i32],
    rel_dy: &[i32],
    rel_dz: &[i32],
    rel_packed: &[u16],
    rel_mask: &[u16],
    rel_drip: &[u8],
    mode: SeedMode,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
    tol: u8,
    max_score: i32,
) -> Result<Int32Array, JsValue> {
    let samples = make_samples(rel_dx, rel_dy, rel_dz, rel_packed, rel_mask, rel_drip, mode)?;
    scan_scored_prepared(
        mode,
        &samples,
        x0,
        x1,
        y0,
        y1,
        z0,
        z1,
        max_matches,
        tol,
        max_score,
    )
}

#[wasm_bindgen]
pub struct ScanPlan {
    mode: SeedMode,
    samples: Vec<Sample>,
    strict_y_cache: RefCell<Option<StrictYCache>>,
}

#[wasm_bindgen]
impl ScanPlan {
    #[wasm_bindgen(constructor)]
    pub fn new(
        rel_dx: &[i32],
        rel_dy: &[i32],
        rel_dz: &[i32],
        rel_packed: &[u16],
        rel_mask: &[u16],
        rel_drip: &[u8],
        seed_mode: u8,
    ) -> Result<ScanPlan, JsValue> {
        let mode = SeedMode::from_code(seed_mode)?;
        let samples = make_samples(rel_dx, rel_dy, rel_dz, rel_packed, rel_mask, rel_drip, mode)?;

        Ok(Self {
            mode,
            samples,
            strict_y_cache: RefCell::new(None),
        })
    }

    pub fn scan_strict_box(
        &self,
        x0: i32,
        x1: i32,
        y0: i32,
        y1: i32,
        z0: i32,
        z1: i32,
        max_matches: u32,
    ) -> Result<Int32Array, JsValue> {
        let kind = strict_check_kind(&self.samples);
        {
            let mut cache_slot = self.strict_y_cache.borrow_mut();
            let needs_rebuild = cache_slot
                .as_ref()
                .map(|cache| !strict_y_cache_matches(cache, self.mode, kind, y0, y1))
                .unwrap_or(true);

            if needs_rebuild {
                *cache_slot = build_strict_y_cache(self.mode, kind, &self.samples, y0, y1);
            }

            if let Some(cache) = cache_slot.as_ref() {
                return scan_strict_prepared_with_y_cache(
                    self.mode,
                    &self.samples,
                    cache,
                    x0,
                    x1,
                    y0,
                    y1,
                    z0,
                    z1,
                    max_matches,
                );
            }
        }

        scan_strict_prepared(
            self.mode,
            &self.samples,
            x0,
            x1,
            y0,
            y1,
            z0,
            z1,
            max_matches,
        )
    }

    pub fn scan_scored_box(
        &self,
        x0: i32,
        x1: i32,
        y0: i32,
        y1: i32,
        z0: i32,
        z1: i32,
        max_matches: u32,
        tol: u8,
        max_score: i32,
    ) -> Result<Int32Array, JsValue> {
        scan_scored_prepared(
            self.mode,
            &self.samples,
            x0,
            x1,
            y0,
            y1,
            z0,
            z1,
            max_matches,
            tol,
            max_score,
        )
    }
}

/// Strict scan with explicit seed mode.
///
/// seed_mode:
/// 0 = 1.8+ (Y ignored)
/// 1 = 1.7.10 / vanilla XOR with Y
/// 2 = b1.6-tb3 additive seed with Y
///
/// Returns Int32Array [x,y,z, x,y,z, ...].
#[wasm_bindgen]
pub fn scan_strict_box_seed(
    rel_dx: &[i32],
    rel_dy: &[i32],
    rel_dz: &[i32],
    rel_packed: &[u16],
    rel_mask: &[u16],
    rel_drip: &[u8],
    seed_mode: u8,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
) -> Result<Int32Array, JsValue> {
    scan_strict_impl(
        rel_dx,
        rel_dy,
        rel_dz,
        rel_packed,
        rel_mask,
        rel_drip,
        SeedMode::from_code(seed_mode)?,
        x0,
        x1,
        y0,
        y1,
        z0,
        z1,
        max_matches,
    )
}

/// Scored scan with explicit seed mode.
///
/// Returns Int32Array [x,y,z,score, x,y,z,score, ...].
#[wasm_bindgen]
pub fn scan_scored_box_seed(
    rel_dx: &[i32],
    rel_dy: &[i32],
    rel_dz: &[i32],
    rel_packed: &[u16],
    rel_mask: &[u16],
    rel_drip: &[u8],
    seed_mode: u8,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
    tol: u8,
    max_score: i32,
) -> Result<Int32Array, JsValue> {
    scan_scored_impl(
        rel_dx,
        rel_dy,
        rel_dz,
        rel_packed,
        rel_mask,
        rel_drip,
        SeedMode::from_code(seed_mode)?,
        x0,
        x1,
        y0,
        y1,
        z0,
        z1,
        max_matches,
        tol,
        max_score,
    )
}

/// Legacy strict scan kept for existing callers.
///
/// post1_12_any_y=true maps to the 1.8+ Y-independent hash.
/// false maps to the 1.7.10-style Y-dependent vanilla hash.
#[wasm_bindgen]
pub fn scan_strict_box(
    rel_dx: &[i32],
    rel_dy: &[i32],
    rel_dz: &[i32],
    rel_packed: &[u16],
    rel_mask: &[u16],
    rel_drip: &[u8],
    post1_12_any_y: bool,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
) -> Result<Int32Array, JsValue> {
    scan_strict_impl(
        rel_dx,
        rel_dy,
        rel_dz,
        rel_packed,
        rel_mask,
        rel_drip,
        SeedMode::from_legacy_post_flag(post1_12_any_y),
        x0,
        x1,
        y0,
        y1,
        z0,
        z1,
        max_matches,
    )
}

/// Legacy scored scan kept for existing callers.
#[wasm_bindgen]
pub fn scan_scored_box(
    rel_dx: &[i32],
    rel_dy: &[i32],
    rel_dz: &[i32],
    rel_packed: &[u16],
    rel_mask: &[u16],
    rel_drip: &[u8],
    post1_12_any_y: bool,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    max_matches: u32,
    tol: u8,
    max_score: i32,
) -> Result<Int32Array, JsValue> {
    scan_scored_impl(
        rel_dx,
        rel_dy,
        rel_dz,
        rel_packed,
        rel_mask,
        rel_drip,
        SeedMode::from_legacy_post_flag(post1_12_any_y),
        x0,
        x1,
        y0,
        y1,
        z0,
        z1,
        max_matches,
        tol,
        max_score,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vanilla_i64_reference(x: i32, y: i32, z: i32, ignore_y: bool) -> u16 {
        let yy = if ignore_y { 0 } else { y };
        let x_term = x.wrapping_mul(3_129_871) as i64;
        let z_term = (z as i64).wrapping_mul(116_129_781);
        let mut l = x_term ^ z_term ^ (yy as i64);
        l = l
            .wrapping_mul(l)
            .wrapping_mul(42_317_861)
            .wrapping_add(l.wrapping_mul(11));
        (((l as u64) >> 16) & 0x0fff) as u16
    }

    fn packed_direct(mode: SeedMode, x: i32, y: i32, z: i32) -> u16 {
        let sample = Sample::new(0, 0, 0, 0, 0x0fff, false, mode);
        packed_from_parts(
            mode,
            sample,
            (x as u32).wrapping_mul(X_MULT),
            y,
            (z as u32).wrapping_mul(mode.z_multiplier()),
        )
    }

    #[test]
    fn low32_vanilla_matches_i64_reference() {
        let coords = [
            (0, 0, 0),
            (1, 64, 1),
            (-1, 255, -1),
            (30_000_000, -64, -30_000_000),
            (-12_345, 70, 98_765),
        ];

        for (x, y, z) in coords {
            assert_eq!(
                packed_direct(SeedMode::Post1_8, x, y, z),
                vanilla_i64_reference(x, y, z, true)
            );
            assert_eq!(
                packed_direct(SeedMode::Pre1_8, x, y, z),
                vanilla_i64_reference(x, y, z, false)
            );
        }
    }

    #[test]
    fn post_1_8_ignores_y_but_pre_1_8_does_not() {
        let x = 1234;
        let z = -5678;
        assert_eq!(
            packed_direct(SeedMode::Post1_8, x, 10, z),
            packed_direct(SeedMode::Post1_8, x, 200, z)
        );
        assert_ne!(
            packed_direct(SeedMode::Pre1_8, x, 10, z),
            packed_direct(SeedMode::Pre1_8, x, 200, z)
        );
    }

    #[test]
    fn tb3_uses_distinct_additive_z_multiplier() {
        let vanilla = packed_direct(SeedMode::Pre1_8, 12, 64, 34);
        let tb3 = packed_direct(SeedMode::Beta16Tb3, 12, 64, 34);
        assert_ne!(vanilla, tb3);
        assert_eq!(
            tb3,
            mix_low_12(
                (12i32 as u32)
                    .wrapping_mul(X_MULT)
                    .wrapping_add((34i32 as u32).wrapping_mul(Z_MULT_TB3))
                    .wrapping_add(64u32)
            )
        );
    }

    #[test]
    fn exact_mix_inverse_reconstructs_all_low16_values() {
        for target in [0x000, 0x123, 0x777, 0xfff] {
            let residues = invert_exact_mix_low_12(target);
            assert_eq!(residues.len(), 65_536);

            for &seed_low28 in &residues {
                assert_eq!(mix_low_12(seed_low28), target);
                assert_eq!(seed_low28 & !MIX_INPUT_MASK, 0);
            }
        }
    }

    #[test]
    fn pre_1_8_y_prefilter_matches_bruteforce_candidates() {
        let mode = SeedMode::Pre1_8;
        let rel_dx = [0, -5];
        let rel_dy = [0, 0];
        let rel_dz = [0, -3];
        let rel_packed = [
            packed_direct(mode, 10, 64, -7),
            packed_direct(mode, 5, 64, -10),
        ];
        let rel_mask = [0x0fffu16, 0x0fffu16];
        let rel_drip = [0u8, 0u8];
        let samples = make_samples(
            &rel_dx,
            &rel_dy,
            &rel_dz,
            &rel_packed,
            &rel_mask,
            &rel_drip,
            mode,
        )
        .unwrap();
        let kind = strict_check_kind(&samples);
        let cache = build_strict_y_cache(mode, kind, &samples, 60, 70).unwrap();

        let mut brute = Vec::new();
        for y in 60..=70 {
            for z in -15..=0 {
                let z_seed = (z as u32).wrapping_mul(Z_MULT_VANILLA);
                for x in 0..=20 {
                    let x_seed = (x as u32).wrapping_mul(X_MULT);
                    if candidate_strict_for_code::<SEED_PRE_1_8, STRICT_FULL_MASK>(
                        &samples, x_seed, y, z_seed,
                    ) {
                        brute.push((x, y, z));
                    }
                }
            }
        }

        let mut fast = Vec::new();
        for z in -15..=0 {
            let z_seed = (z as u32).wrapping_mul(Z_MULT_VANILLA);
            for x in 0..=20 {
                let x_seed = (x as u32).wrapping_mul(X_MULT);
                let base = pivot_base_for_code::<SEED_PRE_1_8>(cache.pivot, x_seed, z_seed);
                let mut y_mask = strict_y_cache_y_mask(&cache, base);
                while y_mask != 0 {
                    let y_offset = y_mask.trailing_zeros() as i32;
                    y_mask &= y_mask - 1;
                    let y = cache.y0 + y_offset;

                    if candidate_strict_for_code_skip::<SEED_PRE_1_8, STRICT_FULL_MASK>(
                        &samples,
                        cache.pivot_index,
                        x_seed,
                        y,
                        z_seed,
                    ) {
                        fast.push((x, y, z));
                    }
                }
            }
        }

        brute.sort_unstable();
        fast.sort_unstable();
        assert!(brute.contains(&(10, 64, -7)));
        assert_eq!(fast, brute);
    }
}
