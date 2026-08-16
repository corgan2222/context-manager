//! Lazy icon loading for the table.
//!
//! `ui()` runs many times per second, so nothing expensive may happen in the
//! frame path. A visible row asks the cache for its icon; the cache
//! either has a texture or hands back a placeholder and queues the reference
//! for a worker thread. Nothing blocks, and the list stays scrollable while
//! icons trickle in.

use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use rustc_hash::{FxHashMap, FxHashSet};

use super::extract::{self, Rgba};
use super::parse::{self, IconRef};

/// How many textures may be uploaded per frame.
///
/// Without a cap the first frame after a scan uploads every icon at once and
/// the window visibly freezes. Sixteen fills a screenful in a few frames.
const UPLOADS_PER_FRAME: usize = 16;

pub struct IconCache {
    textures: FxHashMap<String, TextureHandle>,
    /// References already queued, so a row asking every frame enqueues once.
    pending: FxHashSet<String>,
    /// References that could not be extracted. Kept so a broken icon is not
    /// retried on every single frame for the rest of the session.
    failed: FxHashSet<String>,
    requests: Sender<(String, IconRef)>,
    results: Receiver<(String, Option<Rgba>)>,
    placeholder: TextureHandle,
    loaded: usize,
}

impl IconCache {
    pub fn new(ctx: &Context) -> Self {
        let (requests, request_rx) = channel::<(String, IconRef)>();
        let (result_tx, results) = channel::<(String, Option<Rgba>)>();

        let repaint_ctx = ctx.clone();
        std::thread::Builder::new()
            .name("icon-extractor".into())
            .spawn(move || {
                for (key, reference) in request_rx {
                    let pixels = extract::load(&reference);
                    if result_tx.send((key, pixels)).is_err() {
                        // The UI is gone; nothing left to deliver to.
                        break;
                    }
                    // egui sleeps until something happens, so a finished icon
                    // has to wake it or it appears only on the next mouse move.
                    repaint_ctx.request_repaint();
                }
            })
            .expect("icon worker thread");

        Self {
            textures: FxHashMap::default(),
            pending: FxHashSet::default(),
            failed: FxHashSet::default(),
            requests,
            results,
            placeholder: placeholder_texture(ctx),
            loaded: 0,
        }
    }

    /// Called once per visible row. Must stay cheap.
    ///
    /// Returns the placeholder while the worker is busy, so the row height
    /// never jumps when the real icon arrives.
    pub fn get(&mut self, raw_reference: &str) -> &TextureHandle {
        let Some(reference) = parse::parse(raw_reference) else {
            return &self.placeholder;
        };
        let key = reference.cache_key();

        if self.failed.contains(&key) {
            return &self.placeholder;
        }

        // Two lookups rather than one `if let`, because returning a borrow
        // from the first would keep the map borrowed for the whole function.
        if self.textures.contains_key(&key) {
            return &self.textures[&key];
        }

        if self.pending.insert(key.clone()) {
            let _ = self.requests.send((key, reference));
        }
        &self.placeholder
    }

    /// Called once at the top of every frame.
    ///
    /// Uploading is capped; whatever is left waits for the next frame.
    pub fn poll(&mut self, ctx: &Context) {
        for (key, pixels) in self.results.try_iter().take(UPLOADS_PER_FRAME) {
            self.pending.remove(&key);

            match pixels {
                Some(rgba) => {
                    // GDI hands back premultiplied alpha. Using the
                    // unmultiplied constructor here would premultiply a second
                    // time and darken every soft edge.
                    let image = ColorImage::from_rgba_premultiplied(
                        [rgba.width as usize, rgba.height as usize],
                        &rgba.pixels,
                    );
                    let texture = ctx.load_texture(&key, image, TextureOptions::LINEAR);
                    self.textures.insert(key, texture);
                    self.loaded += 1;
                }
                None => {
                    self.failed.insert(key);
                }
            }
        }
    }

    /// Counts for the status bar: loaded, still queued, given up on.
    pub fn stats(&self) -> (usize, usize, usize) {
        (self.loaded, self.pending.len(), self.failed.len())
    }
}

/// A neutral square, shown until the real icon arrives.
///
/// Deliberately not fully transparent: an empty cell that suddenly fills looks
/// like a rendering glitch, a faint square reads as "loading".
fn placeholder_texture(ctx: &Context) -> TextureHandle {
    let size = extract::ICON_SIZE as usize;
    let mut image = ColorImage::filled([size, size], egui::Color32::TRANSPARENT);

    let faint = egui::Color32::from_gray(128).gamma_multiply(0.25);
    for y in 0..size {
        for x in 0..size {
            let border = x == 0 || y == 0 || x == size - 1 || y == size - 1;
            if border {
                image[(x, y)] = faint;
            }
        }
    }

    ctx.load_texture("icon-placeholder", image, TextureOptions::NEAREST)
}
