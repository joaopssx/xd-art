//! Building palettes from images.
//!
//! The k-means here is the original program's: integer RGB distance, eight iterations,
//! integer means, seeded with k-means++ through xorshift128+. The one change is the
//! empty cluster case, where the original picks a replacement with C's `rand()`. That
//! depends on the C library and on thread timing, so xd-art draws from its own seeded
//! generator instead and stays deterministic.

/// The original's xorshift128+ generator, seeded through murmur3's finalizer.
#[derive(Clone, Copy, Debug)]
pub struct Rng([u64; 2]);

fn avalanche(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    h
}

impl Rng {
    /// Seeds the generator.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let a = avalanche((seed << 1) | 1);
        Self([a, avalanche(a)])
    }

    /// Next value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0[0];
        let y = self.0[1];
        self.0[0] = y;
        x ^= x << 23;
        x ^= x >> 17;
        x ^= y ^ (y >> 26);
        self.0[1] = x;
        x.wrapping_add(y)
    }
}

#[inline]
fn dist(a: [u8; 3], b: [u8; 3]) -> u64 {
    let d = |i: usize| {
        let v = i32::from(a[i]) - i32::from(b[i]);
        (v * v) as u64
    };
    d(0) + d(1) + d(2)
}

/// Scratch space for [`kmeans`], so repeated calls do not allocate.
#[derive(Debug, Default)]
pub struct Scratch {
    distance: Vec<u64>,
    centers: Vec<[u8; 3]>,
    sums: Vec<[u64; 3]>,
    counts: Vec<u64>,
}

/// Picks the starting centers. With `plus_plus`, each new center is drawn with
/// probability proportional to its squared distance from the nearest center so far.
fn choose_centers(px: &[[u8; 3]], k: usize, rng: &mut Rng, plus_plus: bool, s: &mut Scratch) {
    let n = px.len() as u64;
    s.centers.clear();
    if !plus_plus {
        for _ in 0..k {
            s.centers.push(px[(rng.next_u64() % n) as usize]);
        }
        return;
    }
    s.centers.push(px[(rng.next_u64() % n) as usize]);
    s.distance.clear();
    s.distance.resize(px.len(), u64::MAX);
    for _ in 1..k {
        let last = s.centers[s.centers.len() - 1];
        let mut sum = 0u64;
        for (d, &p) in s.distance.iter_mut().zip(px) {
            *d = (*d).min(dist(p, last));
            sum += *d;
        }
        let target = if sum == 0 { 0 } else { rng.next_u64() % sum };
        let mut acc = 0u64;
        let pick = s.distance.iter().position(|&d| {
            acc += d;
            target < acc
        });
        let p = match pick {
            Some(j) => px[j],
            None => px[(rng.next_u64() % n) as usize],
        };
        s.centers.push(p);
    }
}

/// Runs k-means over `px` and leaves the centers and cluster sizes in `s`. Returns the
/// number of centers, which is `k` unless `px` is empty.
fn kmeans(px: &[[u8; 3]], k: usize, seed: u64, plus_plus: bool, s: &mut Scratch) -> usize {
    if px.is_empty() || k == 0 {
        return 0;
    }
    let mut rng = Rng::new(seed);
    choose_centers(px, k, &mut rng, plus_plus, s);
    let n = px.len() as u64;
    for _ in 0..8 {
        s.sums.clear();
        s.sums.resize(k, [0; 3]);
        s.counts.clear();
        s.counts.resize(k, 0);
        for &p in px {
            let mut best = 0;
            let mut best_d = u64::MAX;
            for (c, &center) in s.centers.iter().enumerate() {
                let d = dist(p, center);
                if d < best_d {
                    best_d = d;
                    best = c;
                }
            }
            s.counts[best] += 1;
            for ch in 0..3 {
                s.sums[best][ch] += u64::from(p[ch]);
            }
        }
        for j in 0..k {
            let c = s.counts[j];
            s.centers[j] = if c > 0 {
                s.sums[j].map(|v| (v / c) as u8)
            } else {
                px[(rng.next_u64() % n) as usize]
            };
        }
    }
    k
}

/// The center of the largest cluster after k-means with k-means++ seeding. Used by
/// cluster sampling to find the dominant color of an area.
pub fn largest_cluster(px: &[[u8; 3]], k: usize, seed: u64, s: &mut Scratch) -> [u8; 3] {
    let k = kmeans(px, k, seed, true, s);
    let mut best = s.centers.first().copied().unwrap_or([0; 3]);
    let mut size = 0;
    for j in 0..k {
        if s.counts[j] > size {
            size = s.counts[j];
            best = s.centers[j];
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_matches_reference() {
        // First values of the original's rand_xor seeded with 0xdeadbeef, computed with
        // the C code from shared/rand.c.
        let mut r = Rng::new(0xdead_beef);
        assert_eq!(r.next_u64(), 0x20f5_df82_26c3_fc58);
        assert_eq!(r.next_u64(), 0x934d_117e_5d03_3e0f);
        assert_eq!(r.next_u64(), 0xa7bb_7a42_3bbe_5320);
    }

    #[test]
    fn largest_cluster_finds_the_majority() {
        let mut px = vec![[200, 10, 10]; 30];
        px.extend([[0, 0, 255]; 5]);
        px.extend([[0, 255, 0]; 3]);
        let mut s = Scratch::default();
        assert_eq!(largest_cluster(&px, 3, 0xdead_beef, &mut s), [200, 10, 10]);
    }

    #[test]
    fn degenerate_inputs() {
        let mut s = Scratch::default();
        assert_eq!(largest_cluster(&[], 3, 1, &mut s), [0; 3]);
        assert_eq!(largest_cluster(&[[5, 6, 7]], 3, 1, &mut s), [5, 6, 7]);
        assert_eq!(largest_cluster(&[[9; 3]; 4], 3, 1, &mut s), [9; 3]);
    }
}
