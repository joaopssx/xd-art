//! Post-processing on the indexed image.
//!
//! - Outline and inline, as in the original: a transparent pixel next to an opaque one
//!   (4-neighbourhood) becomes the outline color, an opaque pixel next to a transparent
//!   one becomes the inline color. Both look at the image as it was before either ran.
//! - Merging palette colors that ended up too close to each other.
//! - Removing isolated pixels, the single stray dots dithering and noise leave behind.

use crate::buffer::Indexed;
use crate::settings::Post;

/// Runs every post step the settings ask for, in place.
pub fn apply(ix: &mut Indexed, palette: &[[u8; 3]], p: &Post) {
    if p.merge > 0 {
        merge_similar(ix, palette, p.merge);
    }
    if p.cleanup > 0 {
        cleanup(ix, p.cleanup);
    }
    if p.outline.is_some() || p.inline.is_some() {
        lines(ix, p.outline, p.inline);
    }
}

/// Outline and inline drawing.
pub fn lines(ix: &mut Indexed, outline: Option<u8>, inline: Option<u8>) {
    let (w, h) = (ix.width as i64, ix.height as i64);
    let before = ix.clone();
    let clear = |x: i64, y: i64| before.is_clear((y * w + x) as usize);
    for y in 0..h {
        for x in 0..w {
            let here = clear(x, y);
            let mut opposite = 0;
            for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && nx < w && ny < h && clear(nx, ny) != here {
                    opposite += 1;
                }
            }
            if opposite == 0 {
                continue;
            }
            let i = (y * w + x) as usize;
            match (here, outline, inline) {
                (true, Some(c), _) => {
                    ix.index[i] = c;
                    ix.set_clear(i, false);
                }
                (false, _, Some(c)) => ix.index[i] = c,
                _ => {}
            }
        }
    }
}

/// Folds palette colors closer than `threshold` (euclidean, 8-bit RGB) into the one
/// that covers more pixels. The palette itself stays as it is; only indices change, so
/// exports keep their color order.
pub fn merge_similar(ix: &mut Indexed, palette: &[[u8; 3]], threshold: u8) {
    let mut count = vec![0usize; palette.len()];
    for (i, &c) in ix.index.iter().enumerate() {
        if !ix.is_clear(i) && (c as usize) < count.len() {
            count[c as usize] += 1;
        }
    }
    let limit = u32::from(threshold) * u32::from(threshold);
    let d2 = |a: [u8; 3], b: [u8; 3]| {
        (0..3)
            .map(|k| u32::from(a[k].abs_diff(b[k])).pow(2))
            .sum::<u32>()
    };
    // Most used colors claim their neighbours first.
    let mut order: Vec<usize> = (0..palette.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(count[i]), i));
    let mut target: Vec<u8> = (0..palette.len()).map(|i| i as u8).collect();
    let mut taken = vec![false; palette.len()];
    for &a in &order {
        if taken[a] || count[a] == 0 {
            continue;
        }
        taken[a] = true;
        for &b in &order {
            if !taken[b] && d2(palette[a], palette[b]) < limit {
                taken[b] = true;
                target[b] = a as u8;
            }
        }
    }
    for c in &mut ix.index {
        if let Some(t) = target.get(*c as usize) {
            *c = *t;
        }
    }
}

/// Replaces isolated pixels with the color around them.
///
/// Level 1 only touches a pixel whose four direct neighbours all share one other color.
/// Level 2 also takes a pixel when three of the four agree, unless one of its eight
/// neighbours has its own color: that keeps one pixel wide lines and diagonals, which
/// are detail, not noise.
pub fn cleanup(ix: &mut Indexed, level: u8) {
    let (w, h) = (ix.width as usize, ix.height as usize);
    if w < 3 || h < 3 {
        return;
    }
    let before = ix.clone();
    let at = |x: usize, y: usize| before.index[y * w + x];
    let opaque = |x: usize, y: usize| !before.is_clear(y * w + x);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            if !opaque(x, y) {
                continue;
            }
            let me = at(x, y);
            let n = [(x, y - 1), (x, y + 1), (x - 1, y), (x + 1, y)];
            if n.iter().any(|&(a, b)| !opaque(a, b)) {
                continue;
            }
            let colors = n.map(|(a, b)| at(a, b));
            if colors.contains(&me) {
                continue;
            }
            let replace = if colors.iter().all(|&c| c == colors[0]) {
                Some(colors[0])
            } else if level >= 2 {
                let diag = [
                    (x - 1, y - 1),
                    (x + 1, y - 1),
                    (x - 1, y + 1),
                    (x + 1, y + 1),
                ];
                let lonely = !diag.iter().any(|&(a, b)| at(a, b) == me);
                colors
                    .iter()
                    .copied()
                    .find(|&c| colors.iter().filter(|&&o| o == c).count() >= 3)
                    .filter(|_| lonely)
            } else {
                None
            };
            if let Some(c) = replace {
                ix.index[y * w + x] = c;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: u32, h: u32, cells: &[u8]) -> Indexed {
        let mut ix = Indexed::new(w, h, 0).unwrap();
        for (i, &c) in cells.iter().enumerate() {
            if c == 9 {
                ix.set_clear(i, true);
            } else {
                ix.index[i] = c;
            }
        }
        ix
    }

    #[test]
    fn outline_and_inline_like_the_original() {
        #[rustfmt::skip]
        let mut ix = grid(4, 3, &[
            9, 9, 9, 9,
            9, 1, 1, 9,
            9, 9, 9, 9,
        ]);
        lines(&mut ix, Some(2), Some(3));
        let shown: Vec<u8> = (0..12)
            .map(|i| if ix.is_clear(i) { 9 } else { ix.index[i] })
            .collect();
        #[rustfmt::skip]
        assert_eq!(shown, [
            9, 2, 2, 9,
            2, 3, 3, 2,
            9, 2, 2, 9,
        ]);
    }

    #[test]
    fn cleanup_levels() {
        #[rustfmt::skip]
        let dot = [
            1, 1, 1,
            1, 5, 1,
            1, 1, 1,
        ];
        let mut a = grid(3, 3, &dot);
        cleanup(&mut a, 1);
        assert_eq!(a.index[4], 1);
        #[rustfmt::skip]
        let three = [
            1, 1, 1,
            1, 5, 1,
            1, 2, 1,
        ];
        let mut b = grid(3, 3, &three);
        cleanup(&mut b, 1);
        assert_eq!(b.index[4], 5);
        cleanup(&mut b, 2);
        assert_eq!(b.index[4], 1);
        #[rustfmt::skip]
        let diagonal = [
            5, 1, 1,
            1, 5, 1,
            1, 2, 1,
        ];
        let mut c = grid(3, 3, &diagonal);
        cleanup(&mut c, 2);
        assert_eq!(c.index[4], 5, "diagonal line is kept");
    }

    #[test]
    fn merge_folds_close_colors_into_the_common_one() {
        let pal = [[10, 10, 10], [12, 11, 10], [200, 0, 0]];
        let mut ix = grid(4, 1, &[0, 0, 1, 2]);
        merge_similar(&mut ix, &pal, 5);
        assert_eq!(ix.index, [0, 0, 0, 2]);
        let mut far = grid(2, 1, &[0, 2]);
        merge_similar(&mut far, &pal, 5);
        assert_eq!(far.index, [0, 2]);
    }
}
