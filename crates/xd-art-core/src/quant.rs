//! Building palettes from images.
//!
//! The k-means here is the original program's: integer RGB distance, eight iterations,
//! integer means, seeded with k-means++ through xorshift128+. The one change is the
//! empty cluster case, where the original picks a replacement with C's `rand()`. That
//! depends on the C library and on thread timing, so xd-art draws from its own seeded
//! generator instead and stays deterministic.
//!
//! Median cut is the classic one (Heckbert, 1982), new in xd-art as a palette builder.
//! The original has a median cut too, but as a way to assign palette colors to parts of
//! the image; that one lives in [`crate::dither`].

use crate::buffer::{Work, to8};
use crate::palette::Palette;
use crate::settings::Quantizer;
use crate::{Error, Result};

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

/// Seed used for palette extraction. The original seeds with the current time, so the
/// same image gives a different palette on every run; xd-art uses a fixed seed so
/// recipes reproduce.
pub const SEED: u64 = 0x5eed_f00d;

/// Opaque pixels of a working image as 8-bit RGB. Transparent pixels (alpha below
/// half) would pull the palette toward whatever color they happen to store.
fn opaque_rgb(img: &Work) -> Vec<[u8; 3]> {
    img.px
        .iter()
        .filter(|p| p[3] >= 0x4000)
        .map(|p| [to8(p[0]), to8(p[1]), to8(p[2])])
        .collect()
}

/// The original's k-means palette: `count` centers after eight iterations, in cluster
/// order. Empty clusters come out black, as in the original.
#[must_use]
pub fn kmeans_palette(px: &[[u8; 3]], count: usize, seed: u64, plus_plus: bool) -> Vec<[u8; 3]> {
    let mut s = Scratch::default();
    let k = kmeans(px, count, seed, plus_plus, &mut s);
    (0..k)
        .map(|j| {
            if s.counts[j] > 0 {
                s.centers[j]
            } else {
                [0; 3]
            }
        })
        .collect()
}

/// Median cut: start with one box holding every color, repeatedly split the box with
/// the widest channel range at its median along that channel, until there are `count`
/// boxes or nothing left to split. Each box becomes the mean of its colors.
#[must_use]
pub fn median_cut(px: &[[u8; 3]], count: usize) -> Vec<[u8; 3]> {
    if px.is_empty() || count == 0 {
        return Vec::new();
    }
    let mut colors = px.to_vec();
    // (start, end) ranges into `colors`.
    let mut boxes = vec![(0usize, colors.len())];
    let range = |c: &[[u8; 3]]| {
        let mut lo = [255u8; 3];
        let mut hi = [0u8; 3];
        for p in c {
            for ch in 0..3 {
                lo[ch] = lo[ch].min(p[ch]);
                hi[ch] = hi[ch].max(p[ch]);
            }
        }
        let r = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let ch = (0..3)
            .max_by_key(|&i| (r[i], std::cmp::Reverse(i)))
            .unwrap_or(0);
        (r[ch], ch)
    };
    while boxes.len() < count {
        let best = boxes
            .iter()
            .enumerate()
            .filter(|(_, (a, b))| b - a > 1)
            .map(|(i, &(a, b))| (range(&colors[a..b]).0, i))
            .filter(|(r, _)| *r > 0)
            .max_by_key(|&(r, i)| (r, std::cmp::Reverse(i)));
        let Some((_, i)) = best else { break };
        let (a, b) = boxes[i];
        let (_, ch) = range(&colors[a..b]);
        colors[a..b].sort_by_key(|p| p[ch]);
        let mid = a + (b - a) / 2;
        boxes[i] = (a, mid);
        boxes.push((mid, b));
    }
    boxes
        .iter()
        .map(|&(a, b)| {
            let n = (b - a) as u64;
            let mut sum = [0u64; 3];
            for p in &colors[a..b] {
                for ch in 0..3 {
                    sum[ch] += u64::from(p[ch]);
                }
            }
            sum.map(|s| ((s + n / 2) / n) as u8)
        })
        .collect()
}

/// Builds a palette of up to `count` colors from the opaque pixels of `img`.
///
/// # Errors
///
/// Fails when `count` is not in `1..=256` or the image has no opaque pixel.
pub fn extract(img: &Work, count: u16, method: Quantizer) -> Result<Palette> {
    if !(1..=256).contains(&count) {
        return Err(Error::Invalid {
            name: "palette size",
            reason: "must be in 1..256".into(),
        });
    }
    let px = opaque_rgb(img);
    if px.is_empty() {
        return Err(Error::Invalid {
            name: "palette",
            reason: "image has no opaque pixels".into(),
        });
    }
    let colors = match method {
        Quantizer::KMeans => kmeans_palette(&px, usize::from(count), SEED, true),
        Quantizer::MedianCut => median_cut(&px, usize::from(count)),
    };
    let name = match method {
        Quantizer::KMeans => "k-means",
        Quantizer::MedianCut => "median cut",
    };
    Palette::new(name, colors)
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

    fn two_blobs() -> Vec<[u8; 3]> {
        let mut px = Vec::new();
        let mut r = Rng::new(3);
        for i in 0..400 {
            let n = (r.next_u64() % 9) as u8;
            px.push(if i % 2 == 0 {
                [20 + n, 30, 200 - n]
            } else {
                [230 - n, 220, 10 + n]
            });
        }
        px
    }

    #[test]
    fn kmeans_finds_two_blobs() {
        let mut pal = kmeans_palette(&two_blobs(), 2, SEED, true);
        pal.sort_unstable();
        assert!(pal[0][0] < 40 && pal[0][2] > 180, "{pal:?}");
        assert!(pal[1][0] > 210 && pal[1][2] < 30, "{pal:?}");
    }

    #[test]
    fn median_cut_splits_on_the_widest_channel() {
        let pal = median_cut(&two_blobs(), 2);
        assert_eq!(pal.len(), 2);
        assert!(
            pal.iter().any(|c| c[0] < 40) && pal.iter().any(|c| c[0] > 210),
            "{pal:?}"
        );
        // Asking for more colors than there are distinct values stops early.
        assert_eq!(median_cut(&[[1, 2, 3]; 10], 8), vec![[1, 2, 3]]);
        let grays: Vec<[u8; 3]> = (0..=255).map(|v| [v; 3]).collect();
        assert_eq!(median_cut(&grays, 16).len(), 16);
    }

    #[test]
    fn extract_is_deterministic_and_checks_input() {
        let mut img = Work::new(20, 20, [0; 4]).unwrap();
        for (i, (p, c)) in img.px.iter_mut().zip(two_blobs()).enumerate() {
            let a = if i % 10 == 0 { 0 } else { 0x7f80 };
            *p = [
                u16::from(c[0]) << 7,
                u16::from(c[1]) << 7,
                u16::from(c[2]) << 7,
                a,
            ];
        }
        for m in [Quantizer::KMeans, Quantizer::MedianCut] {
            let a = extract(&img, 4, m).unwrap();
            let b = extract(&img, 4, m).unwrap();
            assert_eq!(a, b);
            assert!(a.len() <= 4);
        }
        assert!(extract(&img, 0, Quantizer::KMeans).is_err());
        let clear = Work::new(2, 2, [0; 4]).unwrap();
        assert!(extract(&clear, 4, Quantizer::MedianCut).is_err());
    }

    #[test]
    fn degenerate_inputs() {
        let mut s = Scratch::default();
        assert_eq!(largest_cluster(&[], 3, 1, &mut s), [0; 3]);
        assert_eq!(largest_cluster(&[[5, 6, 7]], 3, 1, &mut s), [5, 6, 7]);
        assert_eq!(largest_cluster(&[[9; 3]; 4], 3, 1, &mut s), [9; 3]);
    }
}
