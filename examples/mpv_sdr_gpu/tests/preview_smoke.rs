//! Opt-in native libmpv thumbnail acceptance; no desktop or main player is needed.
use futures::{FutureExt, StreamExt};
use iced_winit::core::image;
use mpv_sdr_gpu::preview::{HEIGHT, Outcome, Preview, WIDTH};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(clap::Parser)]
struct Options {
    source: String,
    #[arg(long)]
    libmpv: String,
    #[arg(long)]
    report: PathBuf,
    /// Require a specific unavailable-source diagnostic instead of pixels.
    #[arg(long)]
    expect_unavailable: Option<String>,
    /// Expect explicit rejection of a non-seekable source.
    #[arg(long, conflicts_with = "expect_unavailable")]
    expect_nonseekable: bool,
    /// Known media duration for first/last-frame boundary acceptance.
    #[arg(long)]
    duration: Option<f64>,
    /// Export one frame for an independent color oracle, not motion/cache acceptance.
    #[arg(long)]
    capture_at: Option<f64>,
}

async fn result(
    preview: &Preview,
    events: &mut futures::channel::mpsc::Receiver<()>,
    mut deadline: &mut futures::future::Fuse<futures::channel::oneshot::Receiver<()>>,
) -> Result<Outcome, String> {
    loop {
        if let Some(outcome) = preview.take() {
            return Ok(outcome);
        }
        futures::select! {
            _ = deadline => return Err("thumbnail smoke timed out".into()),
            event = events.next().fuse() => if event.is_none() { return Err("thumbnail notifications closed".into()); },
        }
    }
}

fn pixels(frame: &mpv_sdr_gpu::preview::Frame) -> Result<&[u8], String> {
    let image::Handle::Rgba {
        width,
        height,
        pixels,
        ..
    } = &frame.image
    else {
        return Err("expected RGBA thumbnail".into());
    };
    if (*width, *height) != (WIDTH, HEIGHT)
        || pixels.len() != (WIDTH * HEIGHT * 4) as usize
        || pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] != 255)
    {
        return Err("thumbnail dimensions or opaque alpha are invalid".into());
    }
    Ok(pixels)
}
fn hash(pixels: &[u8]) -> u64 {
    pixels.iter().fold(1469598103934665603, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clap::Parser;
    let options = Options::parse();
    if options
        .duration
        .is_some_and(|duration| !duration.is_finite() || duration <= 0.0)
    {
        return Err("--duration must be finite and positive".into());
    }
    std::fs::create_dir_all(&options.report)?;
    let (preview, mut events) = Preview::new(options.libmpv)?;
    preview.set_media(Some(options.source.clone()));
    let (timer, timeout) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(30));
        let _ = timer.send(());
    });
    let mut deadline = timeout.fuse();
    futures::executor::block_on(async {
        let mut hashes = Vec::new();
        let mut last = None;
        let targets = options
            .capture_at
            .as_ref()
            .map_or(&[8.5, 0.5, 7.0, 3.3][..], std::slice::from_ref);
        for &target in targets {
            let start = Instant::now();
            let requested = preview.request(target).ok_or("no preview source")?;
            let outcome = result(&preview, &mut events, &mut deadline).await?;
            if outcome.requested != requested {
                return Err("stale preview delivered".to_owned());
            }
            if options.expect_unavailable.is_some() || options.expect_nonseekable {
                let error = outcome
                    .result
                    .err()
                    .ok_or("unsupported source produced a thumbnail")?;
                let expected = if options.expect_nonseekable {
                    "not seekable"
                } else {
                    options.expect_unavailable.as_deref().unwrap()
                };
                if !error.contains(expected) {
                    return Err(format!("unexpected rejection: {error}"));
                }
                println!("PASS explicit unsupported source: {error}");
                preview.request(7.0);
                if result(&preview, &mut events, &mut deadline)
                    .await?
                    .result
                    .is_ok()
                {
                    return Err("unsupported source was silently retried".into());
                }
                return Ok(());
            }
            let frame = outcome.result?;
            let bytes = pixels(&frame)?;
            let digest = hash(bytes);
            if hashes.contains(&digest) {
                return Err("moving-video targets returned duplicate pixels".into());
            }
            hashes.push(digest);
            let metadata = format!(
                "request={requested:.3} actual={:.6} elapsed_ms={:.3} rgba_hash={digest:016x} {}x{} color={}\n",
                frame.actual,
                start.elapsed().as_secs_f64() * 1000.0,
                WIDTH,
                HEIGHT,
                frame.color
            );
            print!("{metadata}");
            std::fs::write(options.report.join(format!("{target}.rgba")), bytes)
                .map_err(|error| error.to_string())?;
            std::fs::write(options.report.join(format!("{target}.txt")), metadata)
                .map_err(|error| error.to_string())?;
            if options.capture_at.is_some() {
                return Ok(());
            }
            last = Some(frame);
        }
        let previous = last.unwrap();
        // Replacing A/B with C must not deliver an older result. The deterministic
        // late-publication case is also covered by the mailbox regression test.
        preview.request(6.0);
        preview.request(2.0);
        preview.request(3.3);
        let cached = result(&preview, &mut events, &mut deadline).await?.result?;
        if cached.requested != 3.3 || cached.image.id() != previous.image.id() {
            return Err("latest-target cache identity lost".into());
        }
        preview.request(6.0);
        preview.cancel();
        if preview.take().is_some() {
            return Err("canceled result was delivered".into());
        }
        preview.set_media(Some(options.source.clone()));
        preview.request(3.3);
        let reloaded = result(&preview, &mut events, &mut deadline).await?.result?;
        if reloaded.requested != 3.3
            || reloaded.image.id() == previous.image.id()
            || hash(pixels(&reloaded)?) != hash(pixels(&previous)?)
        {
            return Err("reload reused old-generation pixels or decoded the wrong frame".into());
        }
        if let Some(duration) = options.duration {
            preview.request(0.0);
            let first = result(&preview, &mut events, &mut deadline).await?.result?;
            let requested = preview.request(duration).ok_or("no preview source")?;
            let end = result(&preview, &mut events, &mut deadline).await?.result?;
            if first.requested != 0.0
                || first.actual < 0.0
                || end.requested != requested
                || end.actual <= first.actual
                || end.actual > duration
            {
                return Err(format!(
                    "invalid boundary frames: first={} end={} duration={duration}",
                    first.actual, end.actual
                ));
            }
            pixels(&first)?;
            pixels(&end)?;
            println!(
                "PASS boundary frames: request=0 actual={:.6}; request={requested:.3} actual={:.6}",
                first.actual, end.actual
            );
        }
        preview.request(6.0);
        preview.set_media(None);
        if preview.take().is_some() {
            return Err("unloaded media delivered a result".into());
        }
        preview.set_media(Some(options.source.clone()));
        preview.request(1.0);
        Ok::<(), String>(())
    })?;
    drop(preview);
    if options.expect_unavailable.is_some() || options.expect_nonseekable {
        println!("PASS unsupported-source worker teardown");
    } else if options.capture_at.is_some() {
        println!(
            "PASS frame export and worker teardown; color acceptance requires an independent oracle"
        );
    } else {
        println!(
            "PASS latest request, cancellation, media generation, cache identity and worker teardown"
        );
    }
    Ok(())
}
