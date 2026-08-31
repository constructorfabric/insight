use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub(super) struct TempSampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TempSampler {
    pub(super) fn start(root: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(0));
        let thread_stop = Arc::clone(&stop);
        let thread_peak = Arc::clone(&peak);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                thread_peak.fetch_max(directory_bytes(&root), Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        Self {
            stop,
            peak,
            thread: Some(thread),
        }
    }

    pub(super) fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .unwrap_or_else(|_| panic!("temporary-byte sampler thread must join"));
        }

        self.peak.load(Ordering::Relaxed)
    }
}

fn directory_bytes(root: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };

    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .map(|path| {
            if path.is_dir() {
                directory_bytes(&path)
            } else {
                std::fs::metadata(path).map_or(0, |metadata| metadata.len())
            }
        })
        .sum()
}

pub(super) struct RssSampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RssSampler {
    pub(super) fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(current_rss_bytes()));
        let thread_stop = Arc::clone(&stop);
        let thread_peak = Arc::clone(&peak);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                thread_peak.fetch_max(current_rss_bytes(), Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        Self {
            stop,
            peak,
            thread: Some(thread),
        }
    }

    pub(super) fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .unwrap_or_else(|_| panic!("RSS sampler thread must join"));
        }

        self.peak.load(Ordering::Relaxed)
    }
}

#[cfg(target_os = "linux")]
pub(super) fn current_rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse::<u64>().ok())
        })
        .unwrap_or(0)
        .saturating_mul(1024)
}

#[cfg(target_os = "macos")]
pub(super) fn current_rss_bytes() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_mul(1024)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) const fn current_rss_bytes() -> u64 {
    0
}
