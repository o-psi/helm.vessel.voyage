//! One owned preview worker with a latest-request mailbox and bounded cache.
use super::{
    decode,
    render::{Config, PreparedPreview},
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
};
use uuid::Uuid;

const MAX_IMAGES: usize = 4;

/// Input must come from Helm's validated, owned staged attachments.
/// Intentionally no Debug implementation: image data never belongs in diagnostics.
pub(super) struct Source {
    pub(super) id: Uuid,
    pub(super) sha256: String,
    pub(super) bytes: Vec<u8>,
}

pub(super) struct Batch {
    pub(super) generation: u64,
    pub(super) images: Vec<(Uuid, Result<Arc<PreparedPreview>, String>)>,
}

struct Request {
    generation: u64,
    config: Config,
    sources: Vec<Source>,
}

#[derive(Default)]
struct Mailbox {
    pending: Option<Request>,
    result: Option<Batch>,
    waiting: Vec<Uuid>,
    clear_cache: bool,
}

struct Shared {
    mailbox: Mutex<Mailbox>,
    ready: Condvar,
    generation: AtomicU64,
    stopped: AtomicBool,
}

struct Cached {
    slot: usize,
    id: Uuid,
    sha256: String,
    value: Arc<PreparedPreview>,
}

pub(super) struct Worker {
    config: Config,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(super) fn new(config: Config) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            mailbox: Mutex::new(Mailbox::default()),
            ready: Condvar::new(),
            generation: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("helm-preview".into())
            .spawn(move || run(worker_shared))?;
        Ok(Self {
            config,
            shared,
            thread: Some(thread),
        })
    }

    /// Replaces queued work instead of growing a queue. At most one request is
    /// decoding and one is pending, each with at most 2 MiB of compressed bytes.
    pub(super) fn request(&self, sources: Vec<Source>) -> Result<u64, String> {
        if self.shared.stopped.load(Ordering::Relaxed) {
            return Err("Preview worker stopped".into());
        }
        if sources.len() > MAX_IMAGES
            || sources.iter().any(|source| {
                source.id.is_nil()
                    || source.sha256.len() != 64
                    || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    || source.bytes.is_empty()
                    || source.bytes.len() > decode::MAX_BYTES
            })
            || sources
                .iter()
                .map(|source| source.bytes.len())
                .sum::<usize>()
                > decode::MAX_BYTES
            || sources
                .iter()
                .enumerate()
                .any(|(index, source)| sources[..index].iter().any(|other| source.id == other.id))
        {
            return Err("Preview attachment bounds or identity rejected".into());
        }
        let generation = self
            .shared
            .generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        let mut mailbox = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        mailbox.waiting = sources.iter().map(|source| source.id).collect();
        mailbox.pending = Some(Request {
            generation,
            config: self.config,
            sources,
        });
        mailbox.result = None;
        self.shared.ready.notify_one();
        Ok(generation)
    }

    /// Only the currently requested generation can reach the UI.
    pub(super) fn poll(&self) -> Option<Batch> {
        let mut mailbox = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(result) = mailbox.result.take() {
            mailbox.waiting.clear();
            return (result.generation == self.shared.generation.load(Ordering::Relaxed))
                .then_some(result);
        }
        if !self.shared.stopped.load(Ordering::Relaxed)
            && self.thread.as_ref().is_some_and(JoinHandle::is_finished)
            && !mailbox.waiting.is_empty()
        {
            return Some(Batch {
                generation: self.shared.generation.load(Ordering::Relaxed),
                images: std::mem::take(&mut mailbox.waiting)
                    .into_iter()
                    .map(|id| (id, Err("Preview worker stopped; metadata retained".into())))
                    .collect(),
            });
        }
        None
    }

    pub(super) fn clear(&self) {
        self.shared.generation.fetch_add(1, Ordering::Relaxed);
        let mut mailbox = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        mailbox.pending = None;
        mailbox.result = None;
        mailbox.waiting.clear();
        mailbox.clear_cache = true;
        self.shared.ready.notify_one();
    }

    pub(super) fn reconfigure(&mut self, config: Config) {
        self.clear();
        self.config = config;
    }

    pub(super) fn shutdown(&mut self) -> Result<(), String> {
        self.shared.stopped.store(true, Ordering::Relaxed);
        self.clear();
        if let Some(thread) = self.thread.take() {
            // Decoder calls finish cooperatively; never leave an unowned thread.
            thread
                .join()
                .map_err(|_| "Preview worker cleanup failed".to_owned())?;
        }
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn run(shared: Arc<Shared>) {
    let mut cache = VecDeque::<Cached>::new();
    loop {
        let request = {
            let mut mailbox = shared
                .mailbox
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            loop {
                if shared.stopped.load(Ordering::Relaxed) {
                    return;
                }
                if mailbox.clear_cache {
                    cache.clear();
                    mailbox.clear_cache = false;
                }
                if let Some(request) = mailbox.pending.take() {
                    break request;
                }
                mailbox = shared
                    .ready
                    .wait(mailbox)
                    .unwrap_or_else(|error| error.into_inner());
            }
        };
        cache.retain(|entry| {
            request
                .sources
                .iter()
                .any(|source| source.id == entry.id && source.sha256 == entry.sha256)
        });
        let cancelled = || {
            shared.stopped.load(Ordering::Relaxed)
                || shared.generation.load(Ordering::Relaxed) != request.generation
        };
        let mut images = Vec::with_capacity(request.sources.len());
        for (slot, source) in request.sources.into_iter().enumerate() {
            if cancelled() {
                break;
            }
            let result = if hex::encode(Sha256::digest(&source.bytes)) != source.sha256 {
                Err("Preview attachment integrity rejected".into())
            } else if let Some(entry) = cache.iter().find(|entry| {
                entry.slot == slot && entry.id == source.id && entry.sha256 == source.sha256
            }) {
                Ok(Arc::clone(&entry.value))
            } else {
                decode::thumbnail(&source.bytes, cancelled)
                    .and_then(|image| {
                        if cancelled() {
                            return Err("Preview cancelled".into());
                        }
                        PreparedPreview::new(image, &request.config, slot)
                    })
                    .map(Arc::new)
            };
            if cancelled() {
                break;
            }
            if let Ok(value) = &result
                && !cache.iter().any(|entry| {
                    entry.slot == slot && entry.id == source.id && entry.sha256 == source.sha256
                })
            {
                cache.push_back(Cached {
                    slot,
                    id: source.id,
                    sha256: source.sha256.clone(),
                    value: Arc::clone(value),
                });
                while cache.len() > MAX_IMAGES {
                    cache.pop_front();
                }
            }
            images.push((source.id, result));
        }
        if !cancelled() {
            let mut mailbox = shared
                .mailbox
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !cancelled() {
                mailbox.result = Some(Batch {
                    generation: request.generation,
                    images,
                });
            }
        }
    }
}
