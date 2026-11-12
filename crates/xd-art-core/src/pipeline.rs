//! The stage graph, with dirty tracking and a per-stage cache.
//!
//! A conversion runs seven stages:
//!
//! ```text
//! source -> Blur -> Sample -> Adjust ----------> Map -> Post -> Output
//!                                \-> Palette --/
//! ```
//!
//! Each stage has a key: a hash of its own settings and the keys of the stages it reads.
//! The source is identified by an id the caller picks. When settings change, only the
//! stages whose key changed run again, and those are exactly the stages downstream of the
//! change. A built-in palette does not depend on the image at all, so moving the
//! brightness slider does not rebuild it; an extracted palette does.
//!
//! Besides the current result of every stage, the pipeline keeps a few older results per
//! stage, so flipping a setting back and forth (the usual way of comparing two options)
//! is served from memory. [`CachePolicy`] bounds how much that may hold.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::buffer::{Image, Indexed, Work};
use crate::cancel::Cancel;
use crate::pool::Pool;
use crate::settings::{Fingerprint, Fnv, PaletteSource, Settings};
use crate::{Error, Result};

/// One step of the conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// Blur on the full size source.
    Blur,
    /// Reduce to the target size.
    Sample,
    /// Sharpen and color adjustments on the small image.
    Adjust,
    /// Resolve or build the palette.
    Palette,
    /// Map every pixel to the palette, with or without dithering.
    Map,
    /// Outline, inline and cleanup on the indexed image.
    Post,
    /// Expand to colors and upscale.
    Output,
}

impl Stage {
    /// Every stage, in run order.
    pub const ALL: [Self; 7] = [
        Self::Blur,
        Self::Sample,
        Self::Adjust,
        Self::Palette,
        Self::Map,
        Self::Post,
        Self::Output,
    ];

    /// Lowercase name, used in timings and traces.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Blur => "blur",
            Self::Sample => "sample",
            Self::Adjust => "adjust",
            Self::Palette => "palette",
            Self::Map => "map",
            Self::Post => "post",
            Self::Output => "output",
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of stages, stored as bits.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StageSet(u8);

impl StageSet {
    /// The empty set.
    pub const EMPTY: Self = Self(0);

    /// Adds a stage.
    pub fn insert(&mut self, s: Stage) {
        self.0 |= s.bit();
    }

    /// Whether `s` is in the set.
    #[must_use]
    pub fn contains(self, s: Stage) -> bool {
        self.0 & s.bit() != 0
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The stages in run order.
    pub fn iter(self) -> impl Iterator<Item = Stage> {
        Stage::ALL.into_iter().filter(move |s| self.contains(*s))
    }
}

impl FromIterator<Stage> for StageSet {
    fn from_iter<I: IntoIterator<Item = Stage>>(iter: I) -> Self {
        let mut set = Self::EMPTY;
        for s in iter {
            set.insert(s);
        }
        set
    }
}

impl std::fmt::Debug for StageSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter().map(Stage::name)).finish()
    }
}

/// The result of a stage.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    /// A working image (blur, sample, adjust).
    Work(Work),
    /// The palette colors.
    Palette(Vec<[u8; 3]>),
    /// Palette indices (map, post).
    Indexed(Indexed),
    /// The final image.
    Image(Image),
}

impl Data {
    /// Approximate bytes held by the result.
    #[must_use]
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Work(w) => w.px.len() * 8,
            Self::Palette(p) => p.len() * 3,
            Self::Indexed(i) => i.index.len() + i.clear.len(),
            Self::Image(i) => i.heap_size(),
        }
    }
}

/// The key of every stage for one source and one set of settings.
#[must_use]
pub fn keys(source_id: u64, s: &Settings) -> [u64; 7] {
    let chain = |parents: &[u64], own: &dyn Fingerprint| {
        let mut h = Fnv::default();
        for p in parents {
            h.u64(*p);
        }
        own.feed(&mut h);
        h.finish()
    };
    let blur = chain(&[source_id], &s.blur);
    let sample = chain(&[blur], &s.sample);
    let adjust = chain(&[sample], &s.adjust);
    let palette = match s.colors.palette {
        PaletteSource::Extract { .. } => chain(&[adjust], &s.colors.palette),
        _ => chain(&[], &s.colors.palette),
    };
    let metric = s.colors.metric.name();
    let mask = s.mask.as_ref().map_or(0, crate::settings::Mask::id);
    let map = chain(
        &[adjust, palette, Fnv::default().str(metric).finish(), mask],
        &s.dither,
    );
    let post = chain(&[map], &s.post);
    let output = chain(&[post, palette], &s.output);
    [blur, sample, adjust, palette, map, post, output]
}

/// A hash of the pixels, for callers that have no better id for a source.
#[must_use]
pub fn image_id(img: &Image) -> u64 {
    let mut h = Fnv::default();
    h.u64(u64::from(img.width()))
        .u64(u64::from(img.height()))
        .bytes(img.rgb());
    if let Some(a) = img.alpha() {
        h.bytes(a);
    }
    h.finish()
}

/// Limits for the older results kept per stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CachePolicy {
    /// Older results kept per stage, besides the current one.
    pub entries_per_stage: usize,
    /// Total bytes of older results across all stages.
    pub max_bytes: usize,
    /// Results bigger than this are never kept as older results. A blurred 24 megapixel
    /// photo is 192 MB in the working format, and keeping copies of that around costs
    /// more than recomputing it.
    pub max_entry_bytes: usize,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            entries_per_stage: 3,
            max_bytes: 256 << 20,
            max_entry_bytes: 64 << 20,
        }
    }
}

struct Entry {
    key: u64,
    data: Arc<Data>,
    used: u64,
}

/// What one run did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Stages that were computed.
    pub ran: StageSet,
    /// Stages restored from older results.
    pub restored: StageSet,
    /// Time spent in each computed stage, indexed like [`Stage::ALL`].
    pub times: [Duration; 7],
}

/// Read access to the source and to the stages that already ran.
pub struct Inputs<'a> {
    source: &'a Image,
    outs: [Option<&'a Data>; 7],
    pool: &'a Pool,
    cancel: &'a Cancel,
}

impl Inputs<'_> {
    /// Buffers to build the result in.
    #[must_use]
    pub fn pool(&self) -> &Pool {
        self.pool
    }

    /// The token to check between rows.
    #[must_use]
    pub fn cancel(&self) -> &Cancel {
        self.cancel
    }

    /// The source image.
    #[must_use]
    pub fn source(&self) -> &Image {
        self.source
    }

    /// The working image produced by `stage`.
    ///
    /// # Errors
    ///
    /// Fails when that stage has not run or does not produce a working image.
    pub fn work(&self, stage: Stage) -> Result<&Work> {
        match self.outs[stage as usize] {
            Some(Data::Work(w)) => Ok(w),
            _ => Err(missing(stage)),
        }
    }

    /// The palette.
    ///
    /// # Errors
    ///
    /// Fails when the palette stage has not run.
    pub fn palette(&self) -> Result<&[[u8; 3]]> {
        match self.outs[Stage::Palette as usize] {
            Some(Data::Palette(p)) => Ok(p),
            _ => Err(missing(Stage::Palette)),
        }
    }

    /// The indexed image produced by `stage`.
    ///
    /// # Errors
    ///
    /// Fails when that stage has not run or does not produce indices.
    pub fn indexed(&self, stage: Stage) -> Result<&Indexed> {
        match self.outs[stage as usize] {
            Some(Data::Indexed(i)) => Ok(i),
            _ => Err(missing(stage)),
        }
    }
}

fn missing(stage: Stage) -> Error {
    Error::Invalid {
        name: "pipeline",
        reason: format!("{} has no result", stage.name()),
    }
}

/// The engine's stages, as [`Pipeline::run`] uses them.
///
/// # Errors
///
/// Fails when a stage fails: unknown built-in palette, cancelled run, and so on.
pub fn exec(stage: Stage, inp: &Inputs<'_>, s: &Settings) -> Result<Data> {
    let (pool, cancel) = (inp.pool(), inp.cancel());
    Ok(match stage {
        Stage::Blur => {
            let mut w = pool.work(0, 0, [0; 4]);
            inp.source().write_work(&mut w);
            crate::prep::blur(&mut w, &s.blur, pool, cancel)?;
            Data::Work(w)
        }
        Stage::Sample => Data::Work(crate::sampling::run(
            inp.work(Stage::Blur)?,
            &s.sample,
            pool,
            cancel,
        )?),
        Stage::Adjust => {
            let mut w = pool.copy(inp.work(Stage::Sample)?);
            crate::prep::adjust(&mut w, &s.adjust, pool, cancel)?;
            Data::Work(w)
        }
        Stage::Palette => Data::Palette(match &s.colors.palette {
            PaletteSource::Builtin(name) => crate::palette::builtin(name)
                .ok_or_else(|| Error::Invalid {
                    name: "palette",
                    reason: format!("no built-in palette named {name}"),
                })?
                .colors()
                .to_vec(),
            PaletteSource::Colors(c) => c.clone(),
            PaletteSource::Extract { count, method } => {
                crate::quant::extract(inp.work(Stage::Adjust)?, *count, *method)?
                    .colors()
                    .to_vec()
            }
        }),
        Stage::Map => Data::Indexed(crate::dither::map(
            inp.work(Stage::Adjust)?,
            inp.palette()?,
            s.colors.metric,
            &s.dither,
            s.mask.as_ref(),
            pool,
            cancel,
        )?),
        Stage::Post => {
            let src = inp.indexed(Stage::Map)?;
            let mut ix = pool.indexed(src.width, src.height);
            ix.index.copy_from_slice(&src.index);
            ix.clear.clone_from(&src.clear);
            crate::post::apply(&mut ix, inp.palette()?, &s.post);
            Data::Indexed(ix)
        }
        Stage::Output => {
            let img = inp.indexed(Stage::Post)?.to_image(inp.palette()?)?;
            Data::Image(img.upscale(s.output.scale)?)
        }
    })
}

/// Runs stages and remembers their results.
pub struct Pipeline {
    policy: CachePolicy,
    current: [Option<(u64, Arc<Data>)>; 7],
    older: [Vec<Entry>; 7],
    tick: u64,
    pool: Pool,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new(CachePolicy::default())
    }
}

impl Pipeline {
    /// An empty pipeline.
    #[must_use]
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            policy,
            current: Default::default(),
            older: Default::default(),
            tick: 0,
            pool: Pool::new(),
        }
    }

    /// Stages that would run for this source and these settings.
    #[must_use]
    pub fn dirty(&self, source_id: u64, s: &Settings) -> StageSet {
        let keys = keys(source_id, s);
        Stage::ALL
            .into_iter()
            .filter(|st| {
                self.current[*st as usize].as_ref().map(|c| c.0) != Some(keys[*st as usize])
            })
            .collect()
    }

    /// Runs every stage that is out of date with the engine's own stages.
    ///
    /// # Errors
    ///
    /// Fails on invalid settings, when a stage fails, or when the run is cancelled.
    pub fn run(
        &mut self,
        source: &Image,
        source_id: u64,
        s: &Settings,
        cancel: &Cancel,
    ) -> Result<Report> {
        self.run_with(source, source_id, s, cancel, exec)
    }

    /// Runs every stage that is out of date, calling `exec` for each one that is not in
    /// the cache either.
    ///
    /// # Errors
    ///
    /// Fails on invalid settings or when `exec` fails. Stages that finished before the
    /// failure keep their results.
    pub fn run_with<F>(
        &mut self,
        source: &Image,
        source_id: u64,
        s: &Settings,
        cancel: &Cancel,
        mut exec: F,
    ) -> Result<Report>
    where
        F: FnMut(Stage, &Inputs<'_>, &Settings) -> Result<Data>,
    {
        s.validate()?;
        let keys = keys(source_id, s);
        let mut report = Report::default();
        for stage in Stage::ALL {
            let i = stage as usize;
            let key = keys[i];
            if self.current[i].as_ref().is_some_and(|c| c.0 == key) {
                continue;
            }
            self.tick += 1;
            let tick = self.tick;
            cancel.check()?;
            let data = if let Some(pos) = self.older[i].iter().position(|e| e.key == key) {
                report.restored.insert(stage);
                self.older[i].swap_remove(pos).data
            } else {
                let inputs = Inputs {
                    source,
                    outs: std::array::from_fn(|j| self.current[j].as_ref().map(|c| &*c.1)),
                    pool: &self.pool,
                    cancel,
                };
                let start = Instant::now();
                let data = exec(stage, &inputs, s)?;
                report.times[i] = start.elapsed();
                report.ran.insert(stage);
                Arc::new(data)
            };
            if let Some((old_key, old)) = self.current[i].replace((key, data)) {
                self.keep(i, old_key, old, tick);
            }
        }
        Ok(report)
    }

    fn keep(&mut self, i: usize, key: u64, data: Arc<Data>, used: u64) {
        let p = self.policy;
        if p.entries_per_stage == 0 || data.heap_size() > p.max_entry_bytes {
            self.recycle(data);
            return;
        }
        let list = &mut self.older[i];
        list.push(Entry { key, data, used });
        if list.len() > p.entries_per_stage {
            let oldest = list
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.used)
                .map(|(j, _)| j);
            if let Some(j) = oldest {
                let e = list.swap_remove(j);
                self.recycle(e.data);
            }
        }
        while self.older_bytes() > p.max_bytes {
            let oldest = self
                .older
                .iter()
                .enumerate()
                .flat_map(|(s, l)| l.iter().enumerate().map(move |(j, e)| (e.used, s, j)))
                .min();
            match oldest {
                Some((_, s, j)) => {
                    let e = self.older[s].swap_remove(j);
                    self.recycle(e.data);
                }
                None => break,
            }
        }
    }

    /// Hands the buffers of a result nobody else holds back to the pool.
    fn recycle(&self, data: Arc<Data>) {
        match Arc::try_unwrap(data) {
            Ok(Data::Work(w)) => self.pool.recycle(w),
            Ok(Data::Indexed(i)) => self.pool.recycle_indexed(i),
            _ => {}
        }
    }

    /// The buffer pool shared by the stages.
    #[must_use]
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Bytes held by older results.
    #[must_use]
    pub fn older_bytes(&self) -> usize {
        self.older
            .iter()
            .flatten()
            .map(|e| e.data.heap_size())
            .sum()
    }

    /// Drops every older result. Current results stay.
    pub fn clear_older(&mut self) {
        let all: Vec<Entry> = self.older.iter_mut().flat_map(std::mem::take).collect();
        for e in all {
            self.recycle(e.data);
        }
    }

    /// The current result of a stage.
    #[must_use]
    pub fn get(&self, stage: Stage) -> Option<&Arc<Data>> {
        self.current[stage as usize].as_ref().map(|c| &c.1)
    }

    /// The final image of the last successful run.
    #[must_use]
    pub fn output(&self) -> Option<&Image> {
        match self.get(Stage::Output).map(|d| &**d) {
            Some(Data::Image(i)) => Some(i),
            _ => None,
        }
    }

    /// The palette of the last successful run.
    #[must_use]
    pub fn palette(&self) -> Option<&[[u8; 3]]> {
        match self.get(Stage::Palette).map(|d| &**d) {
            Some(Data::Palette(p)) => Some(p),
            _ => None,
        }
    }

    /// The post-processed indices of the last successful run.
    #[must_use]
    pub fn indexed(&self) -> Option<&Indexed> {
        match self.get(Stage::Post).map(|d| &**d) {
            Some(Data::Indexed(i)) => Some(i),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{DitherMode, Quantizer};

    fn fake(stage: Stage, inp: &Inputs<'_>, s: &Settings) -> Result<Data> {
        Ok(match stage {
            Stage::Blur => Data::Work(inp.source().to_work()),
            Stage::Sample => Data::Work(inp.pool().copy(inp.work(Stage::Blur)?)),
            Stage::Adjust => Data::Work(inp.pool().copy(inp.work(Stage::Sample)?)),
            Stage::Palette => Data::Palette(match &s.colors.palette {
                PaletteSource::Colors(c) => c.clone(),
                _ => vec![[1, 2, 3]],
            }),
            Stage::Map => {
                let w = inp.work(Stage::Adjust)?;
                inp.palette()?;
                Data::Indexed(Indexed::new(w.width, w.height, 0)?)
            }
            Stage::Post => Data::Indexed(inp.indexed(Stage::Map)?.clone()),
            Stage::Output => Data::Image(inp.indexed(Stage::Post)?.to_image(inp.palette()?)?),
        })
    }

    fn set(stages: &[Stage]) -> StageSet {
        stages.iter().copied().collect()
    }

    fn run(p: &mut Pipeline, img: &Image, id: u64, s: &Settings) -> Report {
        let dirty = p.dirty(id, s);
        let r = p.run_with(img, id, s, &Cancel::new(), fake).unwrap();
        let mut both = r.ran;
        for st in r.restored.iter() {
            both.insert(st);
        }
        assert_eq!(both, dirty, "dirty() must predict what runs");
        r
    }

    #[test]
    fn invalidates_exactly_the_downstream_stages() {
        use Stage::*;
        let img = Image::new(4, 4, [9, 9, 9]).unwrap();
        let mut p = Pipeline::default();
        let mut s = Settings::default();
        assert_eq!(run(&mut p, &img, 1, &s).ran, set(&Stage::ALL));
        assert_eq!(run(&mut p, &img, 1, &s).ran, StageSet::EMPTY);
        assert_eq!(p.output().unwrap().get(0, 0), Some([1, 2, 3, 255]));

        s.dither.mode = DitherMode::Bayer4;
        assert_eq!(run(&mut p, &img, 1, &s).ran, set(&[Map, Post, Output]));
        s.adjust.brightness = 0.1;
        assert_eq!(
            run(&mut p, &img, 1, &s).ran,
            set(&[Adjust, Map, Post, Output])
        );
        s.output.scale = 2;
        assert_eq!(run(&mut p, &img, 1, &s).ran, set(&[Output]));
        s.post.outline = Some(0);
        assert_eq!(run(&mut p, &img, 1, &s).ran, set(&[Post, Output]));
        s.colors.metric = crate::settings::Metric::Cie76;
        assert_eq!(run(&mut p, &img, 1, &s).ran, set(&[Map, Post, Output]));
        s.colors.palette = PaletteSource::Colors(vec![[0; 3], [255; 3]]);
        assert_eq!(
            run(&mut p, &img, 1, &s).ran,
            set(&[Palette, Map, Post, Output])
        );
        s.sample.offset_y = 0.25;
        assert_eq!(
            run(&mut p, &img, 1, &s).ran,
            set(&[Sample, Adjust, Map, Post, Output])
        );
        s.blur.radius = 1.0;
        assert_eq!(
            run(&mut p, &img, 1, &s).ran,
            set(&[Blur, Sample, Adjust, Map, Post, Output])
        );
        assert_eq!(
            run(&mut p, &img, 2, &s).ran,
            set(&[Blur, Sample, Adjust, Map, Post, Output])
        );
    }

    #[test]
    fn extracted_palette_follows_the_image() {
        use Stage::*;
        let img = Image::new(2, 2, [0; 3]).unwrap();
        let mut p = Pipeline::default();
        let mut s = Settings::default();
        s.colors.palette = PaletteSource::Extract {
            count: 4,
            method: Quantizer::KMeans,
        };
        run(&mut p, &img, 1, &s);
        s.adjust.hue = 30.0;
        assert_eq!(
            run(&mut p, &img, 1, &s).ran,
            set(&[Adjust, Palette, Map, Post, Output])
        );
    }

    #[test]
    fn switching_back_restores_from_cache() {
        let img = Image::new(2, 2, [0; 3]).unwrap();
        let mut p = Pipeline::default();
        let a = Settings::default();
        let mut b = a.clone();
        b.dither.mode = DitherMode::Floyd;
        run(&mut p, &img, 1, &a);
        run(&mut p, &img, 1, &b);
        let r = run(&mut p, &img, 1, &a);
        assert!(r.ran.is_empty());
        assert_eq!(r.restored, set(&[Stage::Map, Stage::Post, Stage::Output]));
        assert!(p.older_bytes() > 0);
        p.clear_older();
        assert_eq!(p.older_bytes(), 0);
        assert_eq!(run(&mut p, &img, 1, &b).ran.iter().count(), 3);
    }

    #[test]
    fn big_results_are_not_kept() {
        let img = Image::new(16, 16, [0; 3]).unwrap();
        let policy = CachePolicy {
            max_entry_bytes: 100,
            ..CachePolicy::default()
        };
        let mut p = Pipeline::new(policy);
        let a = Settings::default();
        let mut b = a.clone();
        b.blur.radius = 2.0;
        run(&mut p, &img, 1, &a);
        run(&mut p, &img, 1, &b);
        let r = run(&mut p, &img, 1, &a);
        assert!(r.ran.contains(Stage::Blur));
        assert!(!r.restored.contains(Stage::Blur));
        assert!(r.restored.contains(Stage::Palette) || !r.ran.contains(Stage::Palette));
    }

    #[test]
    fn byte_budget_is_respected() {
        let img = Image::new(8, 8, [0; 3]).unwrap();
        let policy = CachePolicy {
            entries_per_stage: 8,
            max_bytes: 1200,
            max_entry_bytes: 1 << 20,
        };
        let mut p = Pipeline::new(policy);
        let mut s = Settings::default();
        for i in 0..10 {
            s.blur.radius = 1.0 + i as f32;
            run(&mut p, &img, 1, &s);
            assert!(p.older_bytes() <= 1200);
        }
    }

    #[test]
    fn errors_stop_the_run() {
        let img = Image::new(2, 2, [0; 3]).unwrap();
        let mut p = Pipeline::default();
        let s = Settings::default();
        let err = p.run_with(&img, 1, &s, &Cancel::new(), |st, i, s| {
            if st == Stage::Map {
                Err(Error::Cancelled)
            } else {
                fake(st, i, s)
            }
        });
        assert!(matches!(err, Err(Error::Cancelled)));
        assert!(p.output().is_none());
        assert_eq!(
            p.dirty(1, &s),
            set(&[Stage::Map, Stage::Post, Stage::Output])
        );
        let mut bad = s.clone();
        bad.output.scale = 0;
        assert!(p.run_with(&img, 1, &bad, &Cancel::new(), fake).is_err());
    }

    #[test]
    fn cancelled_run_keeps_the_previous_result() {
        let img = Image::new(4, 4, [9, 9, 9]).unwrap();
        let mut p = Pipeline::default();
        let mut s = Settings::default();
        p.run_with(&img, 1, &s, &Cancel::new(), fake).unwrap();
        let before = p.output().unwrap().clone();
        s.colors.palette = PaletteSource::Colors(vec![[7, 7, 7]]);
        let cancel = Cancel::new();
        let r = p.run_with(&img, 1, &s, &cancel, |st, i, s| {
            if st == Stage::Map {
                cancel.cancel();
                i.cancel().check()?;
            }
            fake(st, i, s)
        });
        assert!(matches!(r, Err(Error::Cancelled)));
        assert_eq!(p.output(), Some(&before));
        let dirty = p.dirty(1, &s);
        assert!(dirty.contains(Stage::Map) && !dirty.contains(Stage::Palette));
    }

    #[test]
    fn rapid_changes_from_another_thread() {
        // A worker keeps restarting with new settings while the old run is cancelled, as
        // the interface does while a slider moves. Every finished run must match its own
        // settings, never an earlier one.
        let img = Image::new(8, 8, [0; 3]).unwrap();
        let mut p = Pipeline::default();
        let mut last = None;
        for step in 0..50u8 {
            let mut s = Settings::default();
            s.colors.palette = PaletteSource::Colors(vec![[step, 0, 0]]);
            let cancel = Cancel::new();
            if let Some(prev) = last.replace(cancel.clone()) {
                Cancel::cancel(&prev);
            }
            if step % 3 == 0 {
                cancel.cancel();
            }
            match p.run_with(&img, 1, &s, &cancel, fake) {
                Ok(_) => assert_eq!(p.output().unwrap().get(0, 0), Some([step, 0, 0, 255])),
                Err(Error::Cancelled) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }

    #[test]
    fn evicted_results_feed_the_pool() {
        let img = Image::new(8, 8, [0; 3]).unwrap();
        let mut p = Pipeline::new(CachePolicy {
            entries_per_stage: 0,
            ..CachePolicy::default()
        });
        let mut s = Settings::default();
        p.run_with(&img, 1, &s, &Cancel::new(), fake).unwrap();
        assert_eq!(p.pool().held(), 0);
        s.sample.offset_x = 0.5;
        p.run_with(&img, 1, &s, &Cancel::new(), fake).unwrap();
        assert!(p.pool().held() >= 2);
    }

    #[test]
    fn real_stages_end_to_end() {
        let mut img = Image::new(64, 48, [0; 3]).unwrap();
        for y in 0..48 {
            for x in 0..64 {
                img.set(
                    x,
                    y,
                    [
                        (x * 4) as u8,
                        (y * 5) as u8,
                        128,
                        if x < 4 { 0 } else { 255 },
                    ],
                );
            }
        }
        let mut s = Settings::default();
        s.sample.size = crate::settings::Size::Absolute {
            width: 16,
            height: 12,
            keep_aspect: false,
        };
        s.output.scale = 2;
        s.dither.mode = DitherMode::Floyd;
        let mut p = Pipeline::default();
        let r = p.run(&img, 1, &s, &Cancel::new()).unwrap();
        assert_eq!(r.ran, set(&Stage::ALL));
        let out = p.output().unwrap();
        assert_eq!((out.width(), out.height()), (32, 24));
        let pico = crate::palette::builtin("pico-8").unwrap();
        for y in 0..out.height() {
            for x in 0..out.width() {
                let px = out.get(x, y).unwrap();
                assert!(px[3] == 0 || pico.colors().contains(&[px[0], px[1], px[2]]));
            }
        }
        assert_eq!(out.get(0, 0).unwrap()[3], 0, "alpha threshold applied");
        s.colors.palette = PaletteSource::Builtin("nope".into());
        assert!(p.run(&img, 1, &s, &Cancel::new()).is_err());
    }

    #[test]
    fn mask_invalidates_only_the_map_stage_on() {
        use Stage::*;
        let img = Image::new(4, 4, [9, 9, 9]).unwrap();
        let mut p = Pipeline::default();
        let mut s = Settings::default();
        run(&mut p, &img, 1, &s);
        s.mask = Some(crate::settings::Mask::new(2, 2, vec![0, 255, 255, 0]).unwrap());
        assert_eq!(run(&mut p, &img, 1, &s).ran, set(&[Map, Post, Output]));
    }

    #[test]
    fn image_id_changes_with_pixels() {
        let mut img = Image::new(2, 2, [0; 3]).unwrap();
        let a = image_id(&img);
        img.set(1, 1, [0, 0, 1, 255]);
        assert_ne!(a, image_id(&img));
        img.set(1, 1, [0, 0, 1, 254]);
        assert_ne!(image_id(&img), a);
    }
}
