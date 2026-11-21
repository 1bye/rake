use std::io;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_profiler::profiler::{self, FrameEvent, FrameTimingCollector};

pub(crate) struct Profiling {
    stop: Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Profiling {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(crate) fn start() -> io::Result<Option<Profiling>> {
    if std::env::var_os("RAKE_EVAL_FRAMES").is_none() {
        return Ok(None);
    }

    let started = Instant::now();
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs_f64()
        * 1000.0;
    profiler::set_trace_enabled(true);
    let mut frames = FrameTimingCollector::new();
    let (stop, done) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("frame-reports".into())
        .spawn(move || {
            loop {
                let running =
                    done.recv_timeout(Duration::from_millis(50)) == Err(RecvTimeoutError::Timeout);
                for event in frames.collect_unseen() {
                    if let FrameEvent::Draw(frame) = event {
                        eprintln!(
                            "frame,{:.3},{:.6}",
                            epoch_ms
                                + frame.draw_start.duration_since(started).as_secs_f64() * 1000.0,
                            frame.draw_duration().as_secs_f64() * 1000.0,
                        );
                    }
                }
                if !running {
                    break;
                }
            }
        })?;

    Ok(Some(Profiling {
        stop,
        worker: Some(worker),
    }))
}
