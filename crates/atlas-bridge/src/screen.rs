//! Capture of the laptop screen (Windows and Mac) and JPEG compression to send it to the headset.
//!
//! On a Mac, Atlas Bridge needs the "Screen Recording" permission (System Settings > Privacy &
//! Security) the first time.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use image::codecs::jpeg::JpegEncoder;

/// Width of the image sent if the headset does not ask for another, and the largest and smallest accepted. The maximum
/// height is 5/8 of the width (a whole 16:10 screen); wider screens end up shorter.
pub const DEFAULT_W: u32 = 1600;
pub const MAX_W: u32 = 2560;
pub const MIN_W: u32 = 960;

/// The maximum height for a maximum width `w`.
pub fn max_h(w: u32) -> u32 {
    w * 5 / 8
}
/// JPEG quality: good for reading text without each image weighing too much.
pub const QUALITY: u8 = 75;

/// The size a `w` x `h` screen is reduced to in order to fit in `max_w` x `max_h` without distorting it (it is never
/// enlarged).
pub fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale = (max_w as f32 / w.max(1) as f32).min(max_h as f32 / h.max(1) as f32).min(1.0);
    (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1))
}

/// Compresses an RGBA image to JPEG, reducing it first if it does not fit in `max_w` wide (and 5/8 of that high).
pub fn encode(rgba: image::RgbaImage, max_w: u32) -> Result<Vec<u8>, String> {
    let (w, h) = rgba.dimensions();
    let max_w = max_w.clamp(MIN_W, MAX_W);
    let (nw, nh) = fit(w, h, max_w, max_h(max_w));
    // `thumbnail` averages the pixels of each area: it is ~2.5 times faster than a triangle filter and the text
    // stays just as sharp or sharper.
    let rgba = if (nw, nh) != (w, h) { image::imageops::thumbnail(&rgba, nw, nh) } else { rgba };
    let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, QUALITY).encode_image(&rgb).map_err(|e| format!("jpeg: {e}"))?;
    Ok(out)
}

/// Captures the main screen and returns it as a JPEG at most `max_w` wide.
pub fn capture_jpeg(max_w: u32) -> Result<Vec<u8>, String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("screen capture: {e}"))?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or(monitors.first())
        .ok_or("screen capture: no screen found")?;
    let image = monitor.capture_image().map_err(|e| format!("screen capture: {e}"))?;
    encode(image, max_w)
}

/// Minimum time between the start of two captures (at most ~15 images per second).
pub const MIN_GAP: Duration = Duration::from_millis(66);

/// An image already captured (or the capture error).
type Slot = Arc<(Mutex<Option<Result<Vec<u8>, String>>>, Condvar)>;

/// Captures the screen on its own thread, one image ahead: while one is sent to the headset, the next one is already
/// being captured and compressed. It only captures once the previous one was collected (no wasted CPU). It stops
/// when dropped.
pub struct Grabber {
    slot: Slot,
    stop: Arc<AtomicBool>,
    /// Width the headset asks for (it changes with how big the screen looks there).
    width: Arc<AtomicU32>,
}

impl Grabber {
    pub fn start() -> Self {
        let slot: Slot = Arc::new((Mutex::new(None), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let width = Arc::new(AtomicU32::new(DEFAULT_W));
        let (s, st, wd) = (slot.clone(), stop.clone(), width.clone());
        std::thread::spawn(move || {
            let (lock, ready) = &*s;
            while !st.load(Ordering::Relaxed) {
                // Waits for the previous image to be collected.
                {
                    let guard = lock.lock().unwrap();
                    let (guard, _) = ready.wait_timeout_while(guard, Duration::from_millis(200), |f| f.is_some()).unwrap();
                    if guard.is_some() {
                        continue;
                    }
                }
                let t = Instant::now();
                let frame = capture_jpeg(wd.load(Ordering::Relaxed));
                let failed = frame.is_err();
                *lock.lock().unwrap() = Some(frame);
                ready.notify_all();
                if failed {
                    return;
                }
                if let Some(rest) = MIN_GAP.checked_sub(t.elapsed()) {
                    std::thread::sleep(rest);
                }
            }
        });
        Self { slot, stop, width }
    }

    /// The width the headset asks for (0: the usual one). It applies from the next capture.
    pub fn set_width(&self, w: u32) {
        let w = if w == 0 { DEFAULT_W } else { w.clamp(MIN_W, MAX_W) };
        self.width.store(w, Ordering::Relaxed);
    }

    /// The next image, waiting at most `timeout`. `None` if it is not ready yet.
    pub fn next(&self, timeout: Duration) -> Option<Result<Vec<u8>, String>> {
        let (lock, ready) = &*self.slot;
        let guard = lock.lock().unwrap();
        let (mut guard, _) = ready.wait_timeout_while(guard, timeout, |f| f.is_none()).unwrap();
        let frame = guard.take();
        ready.notify_all();
        frame
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.slot.1.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_screens_shrink_to_fit_keeping_their_shape_and_small_ones_stay() {
        assert_eq!(fit(3840, 2160, 1600, max_h(1600)), (1600, 900));
        assert_eq!(fit(3440, 1440, 1600, max_h(1600)), (1600, 670));
        assert_eq!(fit(2880, 1800, 1600, max_h(1600)), (1600, 1000));
        assert_eq!(fit(1280, 800, 1600, max_h(1600)), (1280, 800), "it is not enlarged");
        assert_eq!(fit(3440, 1440, MAX_W, max_h(MAX_W)), (2560, 1072), "with the maximum, an ultrawide");
    }

    #[test]
    fn a_screen_image_becomes_a_jpeg_that_decodes_back_at_the_fitted_size() {
        let img = image::RgbaImage::from_fn(3200, 2000, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]));
        let jpeg = encode(img.clone(), DEFAULT_W).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "empieza como un JPEG");
        let back = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (1600, 1000));
        // Whatever the headset asks for, between the minimum and the maximum.
        let big = image::load_from_memory_with_format(&encode(img.clone(), 2560).unwrap(), image::ImageFormat::Jpeg).unwrap();
        assert_eq!((big.width(), big.height()), (2560, 1600));
        let huge = image::load_from_memory_with_format(&encode(img, 9999).unwrap(), image::ImageFormat::Jpeg).unwrap();
        assert_eq!(huge.width(), MAX_W, "never more than the maximum");
        assert!(jpeg.len() < 1_000_000, "it weighs less than 1 MB: {}", jpeg.len());
    }
}

#[cfg(test)]
mod bench {
    /// `cargo test --release -p atlas-bridge bench_ -- --ignored --nocapture`: how long each step takes on this laptop.
    #[test]
    #[ignore]
    fn bench_capture_and_encode() {
        use std::time::Instant;
        let monitors = xcap::Monitor::all().unwrap();
        let m = monitors.iter().find(|m| m.is_primary().unwrap_or(false)).unwrap();
        for _ in 0..5 {
            let t = Instant::now();
            let img = m.capture_image().unwrap();
            let cap = t.elapsed();
            let (w, h) = img.dimensions();
            let t = Instant::now();
            let jpeg = super::encode(img, super::MAX_W).unwrap();
            println!("{w}x{h}: capture {cap:?}, reduce and compress {:?}, {} KB", t.elapsed(), jpeg.len() / 1024);
        }
        let g = super::Grabber::start();
        let t = Instant::now();
        let mut n = 0;
        while t.elapsed().as_secs_f32() < 3.0 {
            if let Some(Ok(_)) = g.next(std::time::Duration::from_millis(500)) {
                n += 1;
            }
        }
        println!("Grabber: {:.1} images per second", n as f32 / t.elapsed().as_secs_f32());
    }
}
