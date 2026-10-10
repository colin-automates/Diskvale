//! Bounded caches for expensive, immutable scan-derived view data.
use super::*;
use std::collections::VecDeque;

pub(super) type TypeTotals = Arc<[(String, u64, u64)]>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RowsKey {
    pub scope: usize,
    pub metric: Metric,
    pub query: String,
    pub category: Option<Category>,
    pub by_modified: bool,
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TypesKey {
    pub scope: usize,
    pub metric: Metric,
    pub query: String,
    pub category: Option<Category>,
}

pub(super) struct ResultCache<K, V> {
    ready: VecDeque<(K, V)>,
    job: Option<(K, mpsc::Receiver<V>)>,
}

impl<K, V> Default for ResultCache<K, V> {
    fn default() -> Self {
        Self {
            ready: VecDeque::new(),
            job: None,
        }
    }
}

impl<K: Clone + Eq, V: Clone + Send + 'static> ResultCache<K, V> {
    pub fn poll(&mut self) {
        let outcome = self.job.as_ref().map(|(_, receiver)| receiver.try_recv());
        match outcome {
            Some(Ok(value)) => {
                let (key, _) = self.job.take().unwrap();
                self.store(key, value);
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.job = None;
            }
            _ => {}
        }
    }

    fn store(&mut self, key: K, value: V) {
        self.ready.retain(|(cached, _)| *cached != key);
        self.ready.push_front((key, value));
        // Two file lists share their Arc storage with the table. No whole-scan
        // copies, no unbounded query history, and only one worker per cache.
        self.ready.truncate(2);
    }

    pub fn request(
        &mut self,
        key: K,
        background: bool,
        ctx: &egui::Context,
        build: impl FnOnce() -> V + Send + 'static,
    ) -> Option<V> {
        self.poll();
        if let Some(index) = self.ready.iter().position(|(cached, _)| *cached == key) {
            let entry = self.ready.remove(index).unwrap();
            let value = entry.1.clone();
            self.ready.push_front(entry);
            return Some(value);
        }
        if self.job.is_none() {
            if !background {
                let value = build();
                self.store(key, value.clone());
                return Some(value);
            }
            let (sender, receiver) = mpsc::channel();
            self.job = Some((key, receiver));
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = sender.send(build());
                ctx.request_repaint();
            });
        }
        // A new key waits behind the active job instead of spawning another
        // full-tree traversal on every keystroke. Its latest request is retried
        // next frame, and a completed older key never answers the new request.
        ctx.request_repaint_after(Duration::from_millis(30));
        None
    }

    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.job.is_some()
    }
}

pub(super) fn file_rows(scan: &Scan, key: &RowsKey) -> Arc<[usize]> {
    let predicate = Query::parse(&key.query);
    let mut rows = vec![];
    let mut stack = vec![key.scope];
    while let Some(id) = stack.pop() {
        let node = &scan.nodes[id];
        if node.directory {
            stack.extend(node.children.iter().copied());
        } else if predicate.matches(&node.name, node.size(key.metric), node.modified)
            && key
                .category
                .is_none_or(|category| Category::from_name(&node.name) == category)
        {
            rows.push(id);
        }
    }
    let value = |id: usize| {
        if key.by_modified {
            scan.nodes[id].modified
        } else {
            scan.nodes[id].size(key.metric)
        }
    };
    rows.sort_unstable_by(|a, b| {
        let order = value(*a).cmp(&value(*b));
        let order = if key.descending {
            order.reverse()
        } else {
            order
        };
        order.then_with(|| scan.nodes[*a].name.cmp(&scan.nodes[*b].name))
    });
    rows.into()
}

pub(super) fn type_totals(scan: &Scan, key: &TypesKey) -> TypeTotals {
    let predicate = Query::parse(&key.query);
    scan.extensions_matching(key.scope, key.metric, |node| {
        predicate.matches(&node.name, node.size(key.metric), node.modified)
            && key
                .category
                .is_none_or(|category| Category::from_name(&node.name) == category)
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn cache_reuses_arc_results_and_keeps_only_recent_keys() {
        let ctx = egui::Context::default();
        let mut cache = ResultCache::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let get = |cache: &mut ResultCache<u8, Arc<[usize]>>, key| {
            let calls = calls.clone();
            cache
                .request(key, false, &ctx, move || {
                    calls.fetch_add(1, Ordering::Relaxed);
                    vec![key as usize].into()
                })
                .unwrap()
        };
        let first = get(&mut cache, 1);
        get(&mut cache, 2);
        assert!(Arc::ptr_eq(&first, &get(&mut cache, 1)));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        get(&mut cache, 3);
        assert!(Arc::ptr_eq(&first, &get(&mut cache, 1)));
        get(&mut cache, 2);
        assert_eq!(calls.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn active_job_does_not_block_or_answer_a_different_key() {
        let ctx = egui::Context::default();
        let mut cache = ResultCache::<u8, Arc<[usize]>>::default();
        let (started, startup) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        assert!(
            cache
                .request(1, true, &ctx, move || {
                    started.send(()).unwrap();
                    blocked.recv().unwrap();
                    vec![1].into()
                })
                .is_none()
        );
        startup.recv_timeout(Duration::from_secs(2)).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        assert!(
            cache
                .request(2, true, &ctx, move || {
                    counter.fetch_add(1, Ordering::Relaxed);
                    vec![2].into()
                })
                .is_none()
        );
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "A second worker started behind the blocked job"
        );
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let result = loop {
            let counter = calls.clone();
            if let Some(value) = cache.request(2, true, &ctx, move || {
                counter.fetch_add(1, Ordering::Relaxed);
                vec![2].into()
            }) {
                break value;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(&*result, &[2], "The older job leaked into the current key");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
