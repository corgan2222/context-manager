//! Deriving the menu category of an OpenAPI operation, without knowing the service.
//!
//! An OpenAPI description offers several *axes* along which its operations could
//! be grouped: the declared tag, or any position of the URL path -- counted from
//! the front (`PathPrefix`) or from the operation backwards (`PathSuffix`). None
//! of them is right by definition. So every axis competes, all are measured with
//! the same yardstick, and the winner becomes the menu.
//!
//! The yardstick has four factors, all in `0..=1` except the last:
//!
//! ```text
//!     coverage    share of operations landing in a group worth showing
//!     evenness    H / ln(k)  -- entropy normalised by the group count
//!     fit         2*sqrt(n) / (k + n/k), capped at 1
//!     1 + corr    corroboration bonus from the summaries
//!     score       = coverage * evenness * fit * (1 + corr)
//! ```
//!
//! Why each of them is needed -- every one of these was a measured failure of a
//! simpler formula:
//!
//! * `evenness` normalises by `ln(k)`. Raw entropy grows with `ln(k)`, so a raw
//!   score always prefers the finest axis available: given `/image/resize/sync`
//!   and `/image/resize/async`, it would group by verb (26 drawers of 2) instead
//!   of by medium. Normalised, "5 balanced groups" and "26 balanced groups" score
//!   alike, and the remaining factors decide.
//! * `fit` is the two-level menu cost `k + n/k`, minimal at `k = sqrt(n)`. It
//!   rejects both extremes: a 2-way split of 232 operations, and 219 drawers of
//!   one. Without it, a perfectly balanced but meaningless `sync|async` segment
//!   scores 1.0 and beats a good but lopsided tag.
//! * `corroboration` is the share of operations whose own summary repeats their
//!   group label ("Resize Image" under `image`). A real category is talked about;
//!   a tenant id, a shard name or a version folder is not. It is a bonus and
//!   never a veto, so a description in another language than its paths -- German
//!   summaries, English segments -- merely loses the bonus on both sides.
//! * `coverage` counts only operations in groups of at least [`min_group_size`],
//!   and operations *without* a key on that axis form no group of their own. An
//!   axis that exists for half the service cannot win on that half's tidiness.
//!
//! The tag additionally holds a [`TAG_MARGIN`] handicap: it is the author's
//! declared intent and only loses when a path axis is twice as good. That is what
//! keeps requirement "meaningless paths, good tags" intact, and it is also what
//! stops implementation names, tenant ids and CRUD verbs from taking over.
//!
//! Nothing here knows a single word of any concrete service, and nothing here
//! does I/O.

use std::collections::HashMap;

use super::spec::Tool;

/// Where a candidate grouping reads its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// The declared OpenAPI tag.
    Tag,
    /// The i-th path segment counted from the front, 0-based.
    PathPrefix(usize),
    /// The k-th path segment counted backwards from the last one, 1-based:
    /// `PathSuffix(1)` is the segment directly above the operation.
    ///
    /// Suffix axes are what makes a service with uneven path depth work:
    /// `/api/v1/tools/image/resize` and `/api/image/resize` disagree about every
    /// prefix position but agree that `image` sits above the operation.
    PathSuffix(usize),
}

/// A path axis must beat the tag by this factor before it may replace it.
const TAG_MARGIN: f64 = 2.0;

/// A prefix axis must beat the best suffix axis by this factor before it is
/// preferred. On a service of even depth the two describe the same column and
/// score within a hair of each other -- the suffix then wins, because it is the
/// one that survives a service whose paths are not all equally deep.
const PREFIX_MARGIN: f64 = 1.05;

/// A tag holding more than this share of the service is a container, not a
/// category, and is never used as a fallback label.
const DOMINANT_TAG_SHARE: f64 = 0.5;

/// A group with fewer members than this is noise, not a menu entry:
/// at least two operations, and at least 2 % of the service.
pub fn min_group_size(n: usize) -> usize {
    std::cmp::max(2, n / 50)
}

/// The grouping chosen for one description.
///
/// Build it **once, from the complete description**, and then ask it about every
/// operation. Building it from a filtered view -- the result of a search box, say
/// -- re-runs the competition on that subset, where the category segment is
/// often constant and some incidental axis wins instead. That is a real bug that
/// this type exists to prevent, hence the deliberate split between [`infer`] and
/// [`category_of`].
///
/// [`infer`]: Grouping::infer
/// [`category_of`]: Grouping::category_of
#[derive(Debug, Clone)]
pub struct Grouping {
    axis: Axis,
    /// folded key -> display label, for the groups big enough to show.
    accepted: HashMap<String, String>,
    /// folded tags that swallow most of the service and must not label anything.
    dominant_tags: Vec<String>,
    /// folded word -> the spelling the description itself uses most emphatically.
    spellings: HashMap<String, String>,
    other_label: String,
}

impl Grouping {
    /// Score every axis and keep the winner.
    pub fn infer(all: &[Tool]) -> Grouping {
        let best_of = |axes: &[Axis]| -> (Option<Axis>, f64) {
            axes.iter().fold((None, 0.0), |(best, top), &axis| {
                let s = score(all, axis);
                if s > top {
                    (Some(axis), s)
                } else {
                    (best, top)
                }
            })
        };
        let (suffix, suffix_score) = best_of(&suffix_axes(all));
        let (prefix, prefix_score) = best_of(&prefix_axes(all));

        let (path, path_score) = if prefix_score > suffix_score * PREFIX_MARGIN {
            (prefix, prefix_score)
        } else if suffix.is_some() {
            (suffix, suffix_score)
        } else {
            (prefix, prefix_score)
        };

        let tag_score = score(all, Axis::Tag);
        let axis = match path {
            Some(path) if path_score > tag_score * TAG_MARGIN => path,
            _ => Axis::Tag,
        };

        let spellings = spellings(all);
        Grouping {
            accepted: accepted_labels(all, axis, &spellings),
            dominant_tags: dominant_tags(all),
            spellings,
            axis,
            other_label: "Other".to_string(),
        }
    }

    /// Replace the label of the last-resort bucket (i18n: "Sonstiges").
    pub fn with_other_label(mut self, label: impl Into<String>) -> Grouping {
        self.other_label = label.into();
        self
    }

    /// Which axis won. For diagnostics and tests.
    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// The groups that will appear as menu headers, unsorted.
    pub fn labels(&self) -> Vec<&str> {
        self.accepted.values().map(String::as_str).collect()
    }

    /// The group this operation belongs in. Never empty.
    ///
    /// Operations the winning axis has nothing to say about are not swept into
    /// the tag it just defeated -- that is how a service with one container tag
    /// grows a drawer called "Tools" again. The chain is: the winning axis, then
    /// any other segment of this very path that is already an accepted group,
    /// then the tag (if the tag is a category and not a container), then the
    /// deepest telling segment of the path as a small group of its own, and only
    /// then the leftover bucket.
    pub fn category_of(&self, tool: &Tool) -> String {
        if let Some(hit) = label_of(tool, self.axis).and_then(|l| self.accepted.get(&fold(&l))) {
            return hit.clone();
        }
        // The category may sit at another depth in this particular path:
        // `/api/v1/tools/image/gif-tools/info` still says `image`.
        let segments = segments(&tool.path);
        let above_operation = &segments[..segments.len().saturating_sub(1)];
        for segment in above_operation.iter().rev() {
            if is_name_like(segment)
                && let Some(hit) = self.accepted.get(&fold(segment))
            {
                return hit.clone();
            }
        }
        if let Some(tag) = tool.tag.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            // Reuse the existing spelling so path "files" and tag "Files" are one
            // drawer rather than two.
            if let Some(hit) = self.accepted.get(&fold(tag)) {
                return hit.clone();
            }
            if !self.dominant_tags.contains(&fold(tag)) {
                return pretty(tag, &self.spellings);
            }
        }
        match above_operation.iter().rev().find(|s| is_name_like(s)) {
            Some(segment) => pretty(segment, &self.spellings),
            None => self.other_label.clone(),
        }
    }
}

/// The requested plain signature, for one-off use and tests.
///
/// `all` must be the **complete** description, not a filtered view -- see
/// [`Grouping`]. It re-infers the grouping on every call, so classifying a whole
/// list this way is O(n^2); build one [`Grouping`] instead.
pub fn category_of(tool: &Tool, all: &[Tool]) -> String {
    Grouping::infer(all).category_of(tool)
}

// --- candidates and labels ------------------------------------------------

fn depth(all: &[Tool]) -> usize {
    all.iter()
        .map(|t| segments(&t.path).len())
        .max()
        .unwrap_or(0)
}

fn prefix_axes(all: &[Tool]) -> Vec<Axis> {
    (0..depth(all)).map(Axis::PathPrefix).collect()
}

fn suffix_axes(all: &[Tool]) -> Vec<Axis> {
    (1..depth(all)).map(Axis::PathSuffix).collect()
}

fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// Can this string name a category at all?
///
/// Rejects path parameters (`{id}`, `:id`), pure numbers, and version markers
/// (`v1`, `v2.1`) -- none of them tells the user what a tool does.
fn is_name_like(s: &str) -> bool {
    if s.is_empty() || s.starts_with('{') || s.ends_with('}') || s.starts_with(':') {
        return false;
    }
    if !s.chars().any(char::is_alphabetic) {
        return false;
    }
    // `strip_prefix`, never `&s[1..]`: a tag may start with a multi-byte
    // character ("Ubersicht" with an umlaut), and slicing it would panic.
    match s.strip_prefix(['v', 'V']) {
        Some(rest) if !rest.is_empty() => !rest.chars().all(|c| c.is_ascii_digit() || c == '.'),
        _ => true,
    }
}

/// The raw label of one operation on one axis, if it has one.
fn label_of(tool: &Tool, axis: Axis) -> Option<String> {
    let raw = match axis {
        Axis::Tag => tool.tag.as_deref()?.trim().to_string(),
        Axis::PathPrefix(i) => {
            let segments = segments(&tool.path);
            // The last segment is the operation ("resize"), never the category.
            if i + 1 >= segments.len() {
                return None;
            }
            segments[i].to_string()
        }
        Axis::PathSuffix(k) => {
            let segments = segments(&tool.path);
            if k >= segments.len() {
                return None;
            }
            segments[segments.len() - 1 - k].to_string()
        }
    };
    is_name_like(&raw).then_some(raw)
}

/// Case and separators do not distinguish drawers: `files` and `Files` are one.
fn fold(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// How the description itself spells each word, folded key -> spelling.
///
/// A path is lower case by convention, so `pdf` and `gif-tools` would end up as
/// "Pdf" and "Gif Tools". The summaries are prose and spell the same words the
/// way a human would: the test service writes "PDF" 31 times and "GIF" 16 times.
/// Taking the spelling with the most capitals recovers the acronyms of any
/// service without a dictionary of acronyms.
fn spellings(all: &[Tool]) -> HashMap<String, String> {
    let mut best: HashMap<String, String> = HashMap::new();
    for tool in all {
        for word in tool.summary.split(|c: char| !c.is_alphanumeric()) {
            if word.is_empty() {
                continue;
            }
            best.entry(fold(word))
                .and_modify(|kept| {
                    if capitals(word) > capitals(kept) {
                        *kept = word.to_string();
                    }
                })
                .or_insert_with(|| word.to_string());
        }
    }
    best
}

/// "save-result" -> "Save Result", "pdf" -> "PDF", "optimize-for-web" ->
/// "Optimize For Web".
///
/// A word the summaries spell with capitals keeps that spelling. Otherwise a
/// short word without vowels is treated as an acronym, and everything else is
/// simply capitalised.
fn pretty(s: &str, spellings: &HashMap<String, String>) -> String {
    s.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|word| {
            if let Some(spelled) = spellings.get(&fold(word))
                && capitals(spelled) > capitals(word)
            {
                return spelled.clone();
            }
            if capitals(word) > 0 {
                return word.to_string();
            }
            let acronym =
                word.chars().count() <= 4 && !word.chars().any(|c| "aeiouyAEIOUY".contains(c));
            if acronym {
                return word.to_uppercase();
            }
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// --- scoring --------------------------------------------------------------

/// Group sizes on one axis, keyed by folded label. Operations without a label on
/// this axis are counted nowhere -- they must not form a group.
fn counts(all: &[Tool], axis: Axis) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for tool in all {
        if let Some(label) = label_of(tool, axis) {
            *counts.entry(fold(&label)).or_insert(0) += 1;
        }
    }
    counts
}

/// The labels of one axis that are big enough to become menu headers, mapped to
/// the spelling that will be shown. Among equal labels the spelling with the
/// most capitals wins, so a description writing "PDF" somewhere keeps "PDF".
fn accepted_labels(
    all: &[Tool],
    axis: Axis,
    spellings: &HashMap<String, String>,
) -> HashMap<String, String> {
    let floor = min_group_size(all.len());
    let counts = counts(all, axis);
    let mut best: HashMap<String, String> = HashMap::new();
    for tool in all {
        let Some(raw) = label_of(tool, axis) else {
            continue;
        };
        let key = fold(&raw);
        if counts.get(&key).copied().unwrap_or(0) < floor {
            continue;
        }
        let candidate = pretty(&raw, spellings);
        best.entry(key)
            .and_modify(|kept| {
                if capitals(&candidate) > capitals(kept) {
                    *kept = candidate.clone();
                }
            })
            .or_insert(candidate);
    }
    best
}

fn capitals(s: &str) -> usize {
    s.chars().filter(|c| c.is_uppercase()).count()
}

/// Tags that hold more than [`DOMINANT_TAG_SHARE`] of the service. Such a tag is
/// a container ("Tools" for 225 of 232) and must never label a leftover group.
fn dominant_tags(all: &[Tool]) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for tool in all {
        if let Some(tag) = tool.tag.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            *counts.entry(fold(tag)).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, c)| *c as f64 > all.len() as f64 * DOMINANT_TAG_SHARE)
        .map(|(k, _)| k)
        .collect()
}

/// Share of operations whose own summary repeats their group label as a word.
fn corroboration(all: &[Tool], axis: Axis, kept: &HashMap<String, usize>) -> f64 {
    let (mut hits, mut total) = (0usize, 0usize);
    for tool in all {
        let Some(label) = label_of(tool, axis) else {
            continue;
        };
        let key = fold(&label);
        if !kept.contains_key(&key) {
            continue;
        }
        total += 1;
        if tool
            .summary
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| !word.is_empty() && fold(word) == key)
        {
            hits += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        hits as f64 / total as f64
    }
}

/// How good a menu this axis would make. See the module documentation.
pub fn score(all: &[Tool], axis: Axis) -> f64 {
    let n = all.len();
    if n == 0 {
        return 0.0;
    }
    let floor = min_group_size(n);
    let kept: HashMap<String, usize> = counts(all, axis)
        .into_iter()
        .filter(|(_, c)| *c >= floor)
        .collect();
    let k = kept.len();
    if k < 2 {
        return 0.0;
    }
    let covered: usize = kept.values().sum();
    let coverage = covered as f64 / n as f64;

    let entropy: f64 = kept
        .values()
        .map(|&c| {
            let p = c as f64 / covered as f64;
            -p * p.ln()
        })
        .sum();
    let evenness = entropy / (k as f64).ln();

    let n = n as f64;
    let k = k as f64;
    let fit = (2.0 * n.sqrt() / (k + n / k)).min(1.0);

    coverage * evenness * fit * (1.0 + corroboration(all, axis, &kept))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tool(path: &str, tag: Option<&str>, summary: &str) -> Tool {
        // Only three of a tool's fields matter here; the rest is what it takes
        // to send a file, which the grouping knows nothing about.
        Tool {
            path: path.to_string(),
            base: "/".into(),
            progress: String::new(),
            tag: tag.map(str::to_string),
            summary: summary.to_string(),
            method: "POST".into(),
            description: None,
            file_field: "file".into(),
            settings: crate::service::spec::Settings::None,
            usable: crate::service::spec::Usable::Yes,
        }
    }

    /// Group sizes by label, biggest first, for readable assertions.
    fn histogram(all: &[Tool], grouping: &Grouping) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for t in all {
            *counts.entry(grouping.category_of(t)).or_default() += 1;
        }
        let mut rows: Vec<(String, usize)> = counts.into_iter().collect();
        rows.sort_by_key(|(label, n)| (std::cmp::Reverse(*n), label.clone()));
        rows
    }

    // --- the two requirements that pull in opposite directions --------------

    #[test]
    fn the_grouping_follows_the_path_when_it_says_more_than_the_tags() {
        let mut all: Vec<Tool> = (0..40)
            .map(|i| tool(&format!("/api/v1/tools/image/op{i}"), Some("Tools"), "op"))
            .collect();
        all.extend(
            (0..30).map(|i| tool(&format!("/api/v1/tools/video/op{i}"), Some("Tools"), "op")),
        );
        all.extend(
            (0..20).map(|i| tool(&format!("/api/v1/tools/audio/op{i}"), Some("Tools"), "op")),
        );
        let grouping = Grouping::infer(&all);
        assert_eq!(grouping.axis(), Axis::PathSuffix(1));
        assert_eq!(grouping.category_of(&all[0]), "Image");
        assert_eq!(grouping.category_of(&all[85]), "Audio");
    }

    #[test]
    fn the_grouping_stays_with_the_tags_when_the_paths_say_nothing() {
        let tags = ["Rendering", "Reporting", "Signing"];
        let all: Vec<Tool> = (0..60)
            .map(|i| tool(&format!("/v1/op/{i}"), Some(tags[i % 3]), "do it"))
            .collect();
        let grouping = Grouping::infer(&all);
        assert_eq!(grouping.axis(), Axis::Tag);
        assert_eq!(grouping.category_of(&all[0]), "Rendering");
        assert_eq!(grouping.category_of(&all[2]), "Signing");
    }

    #[test]
    fn a_path_axis_that_merely_ties_with_the_tags_does_not_replace_them() {
        let all: Vec<Tool> = (0..60)
            .map(|i| {
                let c = ["image", "audio", "pdf"][i % 3];
                tool(&format!("/api/{c}/op{i}"), Some(c), "op")
            })
            .collect();
        assert_eq!(Grouping::infer(&all).axis(), Axis::Tag);
    }

    // --- the counter-examples that broke the earlier drafts ------------------

    #[test]
    fn a_variant_level_below_the_category_does_not_turn_the_menu_into_verbs() {
        // /api/v1/image/resize/sync + /async for every tool: the finest axis is
        // the verb, and raw entropy would have picked it.
        let mut all = Vec::new();
        for (medium, verbs) in [
            (
                "image",
                &["resize", "crop", "convert", "compress", "watermark"][..],
            ),
            ("video", &["convert", "trim", "merge", "compress"][..]),
            ("audio", &["convert", "trim", "normalize"][..]),
            ("pdf", &["split", "merge", "convert", "sign"][..]),
        ] {
            for verb in verbs {
                for mode in ["sync", "async"] {
                    all.push(tool(
                        &format!("/api/v1/{medium}/{verb}/{mode}"),
                        Some(medium),
                        &format!("{verb} {medium}"),
                    ));
                }
            }
        }
        let grouping = Grouping::infer(&all);
        assert_eq!(grouping.axis(), Axis::Tag);
        let rows = histogram(&all, &grouping);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], ("Image".to_string(), 10));
    }

    #[test]
    fn a_balanced_but_meaningless_segment_does_not_beat_lopsided_but_good_tags() {
        // sync|async splits the service perfectly in half -- and says nothing.
        let all: Vec<Tool> = (0..232)
            .map(|i| {
                let tag = if i < 200 {
                    "Image"
                } else if i < 220 {
                    "Video"
                } else {
                    "Audio"
                };
                let mode = if i % 2 == 0 { "sync" } else { "async" };
                tool(&format!("/api/v1/{mode}/op{i}"), Some(tag), "op")
            })
            .collect();
        assert_eq!(Grouping::infer(&all).axis(), Axis::Tag);
    }

    #[test]
    fn tenant_segments_and_shard_names_never_become_menu_groups() {
        let all: Vec<Tool> = (0..200)
            .map(|i| {
                let tag = ["Image", "Video", "Audio", "PDF"][i % 4];
                tool(&format!("/v1/tenant{}/op{i}", i % 20), Some(tag), "op")
            })
            .collect();
        let grouping = Grouping::infer(&all);
        assert_eq!(grouping.axis(), Axis::Tag);
        assert!(grouping.labels().contains(&"Image"));
    }

    #[test]
    fn implementation_names_do_not_outrank_good_tags_just_by_being_more_numerous() {
        let all: Vec<Tool> = (0..60)
            .map(|i| {
                let impl_name = [
                    "imagemagick",
                    "ffmpeg",
                    "ghostscript",
                    "libreoffice",
                    "pandoc",
                ][i % 5];
                let tag = ["Bilder", "Video", "Dokumente"][i % 3];
                tool(&format!("/api/v1/{impl_name}/op{i}"), Some(tag), "op")
            })
            .collect();
        assert_eq!(Grouping::infer(&all).axis(), Axis::Tag);
    }

    #[test]
    fn crud_verbs_do_not_replace_the_resources_they_belong_to() {
        let mut all = Vec::new();
        for resource in ["users", "orders", "invoices", "files", "jobs", "reports"] {
            for verb in ["list", "get", "create", "update", "delete"] {
                all.push(tool(
                    &format!("/api/v1/{resource}/{verb}"),
                    Some(resource),
                    &format!("{verb} {resource}"),
                ));
            }
        }
        assert_eq!(Grouping::infer(&all).axis(), Axis::Tag);
    }

    #[test]
    fn more_categories_than_the_square_root_of_the_service_still_group() {
        // The earlier `k <= min(12, sqrt(n))` cut-off silently gave up here and
        // handed all 234 tools back to the container tag.
        let mut all = Vec::new();
        for category in [
            "image", "video", "audio", "pdf", "archive", "text", "data", "ebook", "font", "code",
            "subtitle", "model3d", "sheet",
        ] {
            for i in 0..18 {
                all.push(tool(
                    &format!("/api/v1/tools/{category}/op{i}"),
                    Some("Tools"),
                    "op",
                ));
            }
        }
        let grouping = Grouping::infer(&all);
        assert_eq!(grouping.labels().len(), 13);
        assert_eq!(histogram(&all, &grouping).len(), 13);
    }

    #[test]
    fn a_category_placed_at_different_depths_is_still_one_group() {
        // Half the service migrated to a deeper layout; the tag is useless.
        let mut all = Vec::new();
        for medium in ["image", "video", "audio", "pdf"] {
            for i in 0..15 {
                let path = if i % 2 == 0 {
                    format!("/api/{medium}/op{i}")
                } else {
                    format!("/api/v1/tools/{medium}/op{i}")
                };
                all.push(tool(&path, Some("Tools"), &format!("op{i} {medium}")));
            }
        }
        let grouping = Grouping::infer(&all);
        assert_eq!(grouping.axis(), Axis::PathSuffix(1));
        let rows = histogram(&all, &grouping);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|(_, n)| *n == 15));
    }

    #[test]
    fn a_path_parameter_above_the_category_does_not_hide_the_category() {
        // The earlier `break` on an empty level stopped the search here.
        let all: Vec<Tool> = (0..100)
            .map(|i| {
                let medium = ["image", "video", "audio", "pdf"][i % 4];
                tool(
                    &format!("/v1/{{tenant}}/{medium}/op{i}"),
                    Some("Tools"),
                    &format!("op{i} {medium}"),
                )
            })
            .collect();
        let grouping = Grouping::infer(&all);
        assert_ne!(grouping.axis(), Axis::Tag);
        assert_eq!(histogram(&all, &grouping).len(), 4);
    }

    #[test]
    fn a_section_too_small_for_its_own_group_does_not_revive_the_container_tag() {
        let mut all: Vec<Tool> = (0..60)
            .map(|i| tool(&format!("/api/v1/tools/image/op{i}"), Some("Tools"), "op"))
            .collect();
        all.extend(
            (0..40).map(|i| tool(&format!("/api/v1/tools/video/op{i}"), Some("Tools"), "op")),
        );
        // "text" has one member -- below the floor, and its tag is the container.
        all.push(tool(
            "/api/v1/tools/text/spellcheck",
            Some("Tools"),
            "Spellcheck",
        ));
        let grouping = Grouping::infer(&all);
        assert_eq!(grouping.category_of(all.last().unwrap()), "Text");
    }

    #[test]
    fn a_leftover_keeps_its_tag_when_that_tag_is_a_category_and_not_a_container() {
        let mut all: Vec<Tool> = (0..60)
            .map(|i| tool(&format!("/api/v1/tools/image/op{i}"), Some("Tools"), "op"))
            .collect();
        all.extend(
            (0..40).map(|i| tool(&format!("/api/v1/tools/video/op{i}"), Some("Tools"), "op")),
        );
        all.push(tool("/api/v1/pipeline/execute", Some("Pipelines"), "Run"));
        assert_eq!(
            Grouping::infer(&all).category_of(all.last().unwrap()),
            "Pipelines"
        );
    }

    #[test]
    fn a_path_group_and_a_tag_of_the_same_name_are_one_drawer() {
        let mut all: Vec<Tool> = (0..60)
            .map(|i| tool(&format!("/api/v1/tools/image/op{i}"), Some("Tools"), "op"))
            .collect();
        all.extend(
            (0..20).map(|i| tool(&format!("/api/v1/tools/files/op{i}"), Some("Tools"), "op")),
        );
        all.push(tool("/api/v1/upload", Some("FILES"), "Upload"));
        let grouping = Grouping::infer(&all);
        assert_eq!(
            grouping.category_of(all.last().unwrap()),
            grouping.category_of(&all[60])
        );
    }

    // --- robustness ---------------------------------------------------------

    #[test]
    fn a_tag_starting_with_a_multi_byte_character_does_not_panic() {
        // `&s[1..]` on "Ubersicht" with an umlaut used to abort the program.
        let all: Vec<Tool> = (0..30)
            .map(|i| {
                let tag = ["Übersicht", "Ändern", "日本語"][i % 3];
                tool(&format!("/v1/op/{i}"), Some(tag), "x")
            })
            .collect();
        let grouping = Grouping::infer(&all);
        assert_eq!(grouping.axis(), Axis::Tag);
        assert_eq!(grouping.category_of(&all[0]), "Übersicht");
        assert_eq!(grouping.category_of(&all[2]), "日本語");
    }

    #[test]
    fn an_empty_description_yields_no_panic_and_no_empty_label() {
        let grouping = Grouping::infer(&[]).with_other_label("Sonstiges");
        assert_eq!(grouping.axis(), Axis::Tag);
        assert_eq!(grouping.category_of(&tool("/", None, "")), "Sonstiges");
        assert_eq!(grouping.category_of(&tool("/{id}", None, "")), "Sonstiges");
        assert_eq!(
            grouping.category_of(&tool("/api/v1/thing", None, "")),
            "Api"
        );
    }

    #[test]
    fn version_and_parameter_segments_never_name_a_group() {
        let all: Vec<Tool> = (0..40)
            .map(|i| {
                let medium = ["image", "audio"][i % 2];
                tool(&format!("/v1/{medium}/{{id}}/op{i}"), None, "op")
            })
            .collect();
        let grouping = Grouping::infer(&all);
        let labels = grouping.labels();
        assert!(!labels.contains(&"V1"));
        assert!(labels.contains(&"Image"));
    }

    #[test]
    fn a_service_without_any_tags_still_finds_its_categories() {
        let all: Vec<Tool> = (0..60)
            .map(|i| {
                let medium = ["image", "audio", "pdf"][i % 3];
                tool(
                    &format!("/v1/{medium}/op{i}"),
                    None,
                    &format!("op{i} {medium}"),
                )
            })
            .collect();
        let grouping = Grouping::infer(&all);
        assert_ne!(grouping.axis(), Axis::Tag);
        assert_eq!(histogram(&all, &grouping).len(), 3);
    }

    #[test]
    fn labels_are_spelled_for_a_menu_and_acronyms_stay_upper_case() {
        assert_eq!(spelled("pdf", ""), "PDF");
        assert_eq!(spelled("image", ""), "Image");
        assert_eq!(spelled("save-result", ""), "Save Result");
        assert_eq!(spelled("optimize-for-web", ""), "Optimize For Web");
    }

    #[test]
    fn a_label_is_spelled_the_way_the_summaries_spell_it() {
        // "gif" has a vowel and no rule of orthography makes it an acronym --
        // but the description writes "GIF" in its own prose, so the menu does too.
        assert_eq!(spelled("gif-tools", "Split GIF into frames"), "GIF Tools");
        assert_eq!(spelled("ocr", "Run OCR on a page"), "OCR");
        // Without that evidence the label is merely capitalised -- never wrong,
        // only plainer.
        assert_eq!(spelled("ocr", "Recognise text"), "Ocr");
    }

    /// `pretty` is private; reach it through the public path that uses it.
    fn spelled(segment: &str, summary: &str) -> String {
        let all: Vec<Tool> = (0..10)
            .map(|i| tool(&format!("/api/{segment}/op{i}"), None, summary))
            .collect();
        Grouping::infer(&all).category_of(&all[0])
    }

    /// The test service, rebuilt segment for segment from its own description:
    /// 225 operations under `/api/v1/tools/<section>/<operation>`, of which nine
    /// carry one more level, plus seven strays outside that shape. Every one of
    /// the 225 is tagged "Tools" -- which is exactly what makes the tag useless.
    fn the_test_service() -> Vec<Tool> {
        let mut all = Vec::new();
        for (section, count) in [
            ("image", 89),
            ("video", 52),
            ("pdf", 28),
            ("audio", 26),
            ("files", 21),
        ] {
            for i in 0..count {
                all.push(tool(
                    &format!("/api/v1/tools/{section}/op{i}"),
                    Some("Tools"),
                    &format!("Op{i} {section}"),
                ));
            }
        }
        for (section, item, sub) in [
            ("image", "optimize-for-web", "preview"),
            ("image", "image-enhancement", "analyze"),
            ("image", "passport-photo", "analyze"),
            ("image", "gif-tools", "info"),
            ("image", "remove-background", "effects"),
            ("image", "strip-metadata", "inspect"),
            ("image", "edit-metadata", "inspect"),
            ("pdf", "pdf-to-image", "info"),
            ("pdf", "pdf-to-image", "preview"),
        ] {
            all.push(tool(
                &format!("/api/v1/tools/{section}/{item}/{sub}"),
                Some("Tools"),
                item,
            ));
        }
        for (path, tag, summary) in [
            (
                "/api/v1/pipeline/execute",
                "Pipelines",
                "Execute a pipeline",
            ),
            ("/api/v1/upload", "Files", "Upload an image"),
            ("/api/v1/preview", "Files", "Generate image preview"),
            ("/api/v1/files/upload", "Files", "Save file to library"),
            ("/api/v1/files/save-result", "Files", "Save a result"),
            (
                "/api/v1/preview/generate",
                "Files",
                "Generate upload preview",
            ),
            ("/api/v1/admin/features/import", "Features", "Import bundle"),
        ] {
            all.push(tool(path, Some(tag), summary));
        }
        all
    }

    #[test]
    fn the_test_service_is_grouped_by_medium_and_not_by_its_container_tag() {
        let all = the_test_service();
        assert_eq!(all.len(), 232);
        let grouping = Grouping::infer(&all);
        assert_eq!(grouping.axis(), Axis::PathSuffix(1));
        assert_eq!(
            histogram(&all, &grouping),
            vec![
                ("Image".to_string(), 96),
                ("Video".to_string(), 52),
                ("PDF".to_string(), 30),
                ("Audio".to_string(), 26),
                ("Files".to_string(), 26),
                ("Features".to_string(), 1),
                ("Pipelines".to_string(), 1),
            ]
        );
    }

    #[test]
    fn an_operation_one_level_deeper_still_joins_its_own_section() {
        // "/api/v1/tools/image/gif-tools/info" says `gif-tools` on the winning
        // axis, which is no group -- but `image` stands right next to it.
        let all = the_test_service();
        let grouping = Grouping::infer(&all);
        let deep = all
            .iter()
            .find(|t| t.path.ends_with("/gif-tools/info"))
            .unwrap();
        assert_eq!(grouping.category_of(deep), "Image");
    }

    #[test]
    fn a_filtered_view_is_classified_with_the_grouping_of_the_whole_description() {
        // A search box narrows the list to a handful of PDF tools. Inferring on
        // that subset would find `pdf` constant and crown some other axis; asking
        // the grouping that was built once from everything cannot go wrong.
        let all = the_test_service();
        let grouping = Grouping::infer(&all);
        let hits: Vec<Tool> = all
            .iter()
            .filter(|t| t.path.contains("pdf"))
            .cloned()
            .collect();
        let rows = histogram(&hits, &grouping);
        assert_eq!(rows[0].0, "PDF");
        assert!(rows.iter().all(|(label, _)| label != "Tools"));
    }

    #[test]
    fn the_floor_for_a_group_grows_with_the_service() {
        assert_eq!(min_group_size(0), 2);
        assert_eq!(min_group_size(100), 2);
        assert_eq!(min_group_size(232), 4);
        assert_eq!(min_group_size(1000), 20);
    }
}
