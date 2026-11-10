use crate::fault::{FaultConfig, FaultInjector};
use crate::{EndpointId, SerialEvent};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Notify;

/// Backpressure policy when an endpoint's inbound buffer is full
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackpressurePolicy {
    /// Drop the incoming newest packet, preserving earlier queued packets
    #[default]
    DropNewest,
    /// Evict the oldest queued packet to make room for the new packet
    DropOldest,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum RoutingError {
    #[error("Endpoint {0} not found")]
    EndpointNotFound(EndpointId),
    #[error("Endpoint {0} already registered")]
    EndpointAlreadyRegistered(EndpointId),
    #[error("Endpoint {0} inbound channel is closed")]
    ChannelClosed(EndpointId),
    #[error("Routing edge from {0} to {1} already exists")]
    EdgeAlreadyExists(EndpointId, EndpointId),
    #[error("Routing edge from {0} to {1} not found")]
    EdgeNotFound(EndpointId, EndpointId),
}

/// Thread-safe bounded FIFO queue with configurable backpressure and async notification
pub struct BoundedEventQueue {
    capacity: usize,
    policy: BackpressurePolicy,
    queue: Mutex<VecDeque<SerialEvent>>,
    notify: Notify,
    closed: AtomicBool,
}

impl BoundedEventQueue {
    pub fn new(capacity: usize, policy: BackpressurePolicy) -> Self {
        Self {
            capacity: capacity.max(1),
            policy,
            queue: Mutex::new(VecDeque::with_capacity(capacity.min(1024))),
            notify: Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    /// Pushes an event into the queue. Returns `true` if queued, `false` if dropped via `DropNewest`.
    pub fn push(&self, event: SerialEvent) -> Result<bool, RoutingError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(RoutingError::ChannelClosed(event.source_id));
        }

        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= self.capacity {
            match self.policy {
                BackpressurePolicy::DropNewest => {
                    return Ok(false);
                }
                BackpressurePolicy::DropOldest => {
                    q.pop_front();
                }
            }
        }
        q.push_back(event);
        self.notify.notify_one();
        Ok(true)
    }

    /// Asynchronously awaits and dequeues the next event
    pub async fn recv(&self, endpoint_id: EndpointId) -> Result<SerialEvent, RoutingError> {
        loop {
            // Create the notification future *before* inspecting shared state. `Notified`
            // records the `notify_waiters` epoch at creation, so a concurrent `close()`
            // landing between the check below and the await can no longer be lost (which
            // would leave this receiver blocked forever on a closed queue).
            let notified = self.notify.notified();
            {
                let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(event) = q.pop_front() {
                    return Ok(event);
                }
                if self.closed.load(Ordering::Acquire) {
                    return Err(RoutingError::ChannelClosed(endpoint_id));
                }
            }
            notified.await;
        }
    }

    /// Non-blocking attempt to dequeue the next event
    pub fn try_recv(&self, endpoint_id: EndpointId) -> Result<Option<SerialEvent>, RoutingError> {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(event) = q.pop_front() {
            Ok(Some(event))
        } else if self.closed.load(Ordering::Acquire) {
            Err(RoutingError::ChannelClosed(endpoint_id))
        } else {
            Ok(None)
        }
    }

    pub fn len(&self) -> usize {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }
}

/// Routing edge targeting a destination endpoint with optional fault injector
#[derive(Clone)]
pub struct EdgeEntry {
    pub target: EndpointId,
    pub fault_injector: Option<Arc<Mutex<FaultInjector>>>,
}

/// Read-Copy-Update snapshot table for lock-free routing lookups
#[derive(Default, Clone)]
pub struct RoutingTable {
    pub endpoints: HashMap<EndpointId, Arc<BoundedEventQueue>>,
    pub routes: HashMap<EndpointId, Vec<EdgeEntry>>,
}

/// Lock-free high-speed routing graph supporting 1:1, 1:N splitting, and N:1 atomic merging
pub struct RoutingGraph {
    table: RwLock<Arc<RoutingTable>>,
}

impl Default for RoutingGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl RoutingGraph {
    pub fn new() -> Self {
        Self {
            table: RwLock::new(Arc::new(RoutingTable::default())),
        }
    }

    /// Registers a new endpoint with a bounded inbound buffer and backpressure policy
    pub fn register_endpoint(
        &self,
        id: EndpointId,
        capacity: usize,
        policy: BackpressurePolicy,
    ) -> Result<(), RoutingError> {
        let mut guard = self.table.write().unwrap_or_else(|e| e.into_inner());
        let mut new_table = (**guard).clone();
        if new_table.endpoints.contains_key(&id) {
            return Err(RoutingError::EndpointAlreadyRegistered(id));
        }
        new_table
            .endpoints
            .insert(id, Arc::new(BoundedEventQueue::new(capacity, policy)));
        *guard = Arc::new(new_table);
        Ok(())
    }

    /// Unregisters an endpoint and cleans up its incoming and outgoing routing edges
    pub fn unregister_endpoint(&self, id: EndpointId) -> Result<(), RoutingError> {
        let mut guard = self.table.write().unwrap_or_else(|e| e.into_inner());
        let mut new_table = (**guard).clone();

        if let Some(queue) = new_table.endpoints.remove(&id) {
            queue.close();
        } else {
            return Err(RoutingError::EndpointNotFound(id));
        }

        // Remove outgoing edges
        new_table.routes.remove(&id);

        // Remove incoming edges from other endpoints
        for edges in new_table.routes.values_mut() {
            edges.retain(|e| e.target != id);
        }

        *guard = Arc::new(new_table);
        Ok(())
    }

    pub fn has_endpoint(&self, id: EndpointId) -> bool {
        let guard = self.table.read().unwrap_or_else(|e| e.into_inner());
        guard.endpoints.contains_key(&id)
    }

    /// Connects a directed edge from `from` endpoint to `to` endpoint with optional fault injection
    pub fn add_edge(
        &self,
        from: EndpointId,
        to: EndpointId,
        fault: Option<FaultConfig>,
    ) -> Result<(), RoutingError> {
        let mut guard = self.table.write().unwrap_or_else(|e| e.into_inner());
        let mut new_table = (**guard).clone();

        if !new_table.endpoints.contains_key(&from) {
            return Err(RoutingError::EndpointNotFound(from));
        }
        if !new_table.endpoints.contains_key(&to) {
            return Err(RoutingError::EndpointNotFound(to));
        }

        let edges = new_table.routes.entry(from).or_default();
        if edges.iter().any(|e| e.target == to) {
            return Err(RoutingError::EdgeAlreadyExists(from, to));
        }

        let injector = fault.map(|cfg| Arc::new(Mutex::new(FaultInjector::new(cfg))));
        edges.push(EdgeEntry {
            target: to,
            fault_injector: injector,
        });

        *guard = Arc::new(new_table);
        Ok(())
    }

    /// Removes a directed routing edge between two endpoints
    pub fn remove_edge(&self, from: EndpointId, to: EndpointId) -> Result<(), RoutingError> {
        let mut guard = self.table.write().unwrap_or_else(|e| e.into_inner());
        let mut new_table = (**guard).clone();

        if let Some(edges) = new_table.routes.get_mut(&from) {
            let initial_len = edges.len();
            edges.retain(|e| e.target != to);
            if edges.len() == initial_len {
                return Err(RoutingError::EdgeNotFound(from, to));
            }
        } else {
            return Err(RoutingError::EdgeNotFound(from, to));
        }

        *guard = Arc::new(new_table);
        Ok(())
    }

    /// Dispatches an event across all downstream edges. Operates lock-free on an RCU snapshot.
    /// Returns the number of destination endpoints successfully queued.
    pub fn route(&self, event: SerialEvent) -> Result<usize, RoutingError> {
        // Grab RCU snapshot in ~1 nanosecond
        let snapshot = {
            let guard = self.table.read().unwrap_or_else(|e| e.into_inner());
            Arc::clone(&guard)
        };

        let edges = match snapshot.routes.get(&event.source_id) {
            Some(edges) => edges,
            None => return Ok(0),
        };

        let now_ns = event.timestamp_ns;
        let mut dispatched = 0;

        for edge in edges {
            let dest_queue = match snapshot.endpoints.get(&edge.target) {
                Some(q) => q,
                None => continue,
            };

            if let Some(injector_lock) = &edge.fault_injector {
                let mut injector = injector_lock.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(fault_output) = injector.inject(event.clone(), now_ns) {
                    let delay = fault_output.delay;
                    let out_event = fault_output.event;
                    let queue = Arc::clone(dest_queue);

                    if delay == Duration::ZERO {
                        if queue.push(out_event).unwrap_or(false) {
                            dispatched += 1;
                        }
                    } else {
                        // Non-blocking asynchronous delay delivery. `route` may also be
                        // called from synchronous code (tests, CLI scenario setup) where
                        // no Tokio runtime exists; spawning there would panic, so fall
                        // back to immediate delivery and document the lost delay.
                        match tokio::runtime::Handle::try_current() {
                            Ok(handle) => {
                                handle.spawn(async move {
                                    tokio::time::sleep(delay).await;
                                    let _ = queue.push(out_event);
                                });
                                dispatched += 1;
                            }
                            Err(_) => {
                                if queue.push(out_event).unwrap_or(false) {
                                    dispatched += 1;
                                }
                            }
                        }
                    }
                }
            } else if dest_queue.push(event.clone()).unwrap_or(false) {
                dispatched += 1;
            }
        }

        Ok(dispatched)
    }

    /// Asynchronously awaits and dequeues the next complete frame for the target endpoint.
    /// In N:1 mergers, complete frames are returned atomically with zero byte interleaving.
    pub async fn recv(&self, endpoint_id: EndpointId) -> Result<SerialEvent, RoutingError> {
        let queue = {
            let guard = self.table.read().unwrap_or_else(|e| e.into_inner());
            Arc::clone(
                guard
                    .endpoints
                    .get(&endpoint_id)
                    .ok_or(RoutingError::EndpointNotFound(endpoint_id))?,
            )
        };
        queue.recv(endpoint_id).await
    }

    /// Non-blocking poll to dequeue the next complete frame
    pub fn try_recv(&self, endpoint_id: EndpointId) -> Result<Option<SerialEvent>, RoutingError> {
        let queue = {
            let guard = self.table.read().unwrap_or_else(|e| e.into_inner());
            Arc::clone(
                guard
                    .endpoints
                    .get(&endpoint_id)
                    .ok_or(RoutingError::EndpointNotFound(endpoint_id))?,
            )
        };
        queue.try_recv(endpoint_id)
    }

    /// Current count of queued frames for an endpoint
    pub fn queue_len(&self, endpoint_id: EndpointId) -> Result<usize, RoutingError> {
        let guard = self.table.read().unwrap_or_else(|e| e.into_inner());
        let queue = guard
            .endpoints
            .get(&endpoint_id)
            .ok_or(RoutingError::EndpointNotFound(endpoint_id))?;
        Ok(queue.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameFlags, PinState};
    use bytes::Bytes;

    fn make_event(source: EndpointId, payload: &[u8]) -> SerialEvent {
        SerialEvent {
            timestamp_ns: 1_000_000,
            source_id: source,
            flags: FrameFlags::empty(),
            pins: PinState::empty(),
            payload: Bytes::copy_from_slice(payload),
        }
    }

    #[tokio::test]
    async fn test_1_to_1_routing() {
        let graph = RoutingGraph::new();
        graph
            .register_endpoint(1, 10, BackpressurePolicy::DropNewest)
            .unwrap();
        graph
            .register_endpoint(2, 10, BackpressurePolicy::DropNewest)
            .unwrap();
        graph.add_edge(1, 2, None).unwrap();

        let count = graph.route(make_event(1, b"hello")).unwrap();
        assert_eq!(count, 1);

        let received = graph.recv(2).await.unwrap();
        assert_eq!(&received.payload[..], b"hello");
    }

    #[tokio::test]
    async fn test_1_to_n_splitter_zero_copy() {
        let graph = RoutingGraph::new();
        graph
            .register_endpoint(1, 10, BackpressurePolicy::DropNewest)
            .unwrap();
        graph
            .register_endpoint(2, 10, BackpressurePolicy::DropNewest)
            .unwrap();
        graph
            .register_endpoint(3, 10, BackpressurePolicy::DropNewest)
            .unwrap();
        graph
            .register_endpoint(4, 10, BackpressurePolicy::DropNewest)
            .unwrap();

        graph.add_edge(1, 2, None).unwrap();
        graph.add_edge(1, 3, None).unwrap();
        graph.add_edge(1, 4, None).unwrap();

        let event = make_event(1, b"telemetry-broadcast");
        let orig_ptr = event.payload.as_ptr();

        let dispatched = graph.route(event).unwrap();
        assert_eq!(dispatched, 3);

        for target in [2, 3, 4] {
            let recv = graph.recv(target).await.unwrap();
            assert_eq!(&recv.payload[..], b"telemetry-broadcast");
            // Invariant 3: Zero-copy pointer identity across fan-out
            assert_eq!(recv.payload.as_ptr(), orig_ptr);
        }
    }

    #[tokio::test]
    async fn test_n_to_1_atomic_frame_merger() {
        let graph = Arc::new(RoutingGraph::new());
        graph
            .register_endpoint(10, 100, BackpressurePolicy::DropNewest)
            .unwrap();

        for src in 1..=3 {
            graph
                .register_endpoint(src, 10, BackpressurePolicy::DropNewest)
                .unwrap();
            graph.add_edge(src, 10, None).unwrap();
        }

        // Concurrently push frames from endpoints 1, 2, 3 into destination 10
        let mut handles = Vec::new();
        for src in 1..=3 {
            let g = Arc::clone(&graph);
            handles.push(tokio::spawn(async move {
                for seq in 0..10 {
                    let msg = format!("SRC_{}_{}", src, seq);
                    g.route(make_event(src, msg.as_bytes())).unwrap();
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let mut received = Vec::new();
        for _ in 0..30 {
            let evt = graph.recv(10).await.unwrap();
            let s = String::from_utf8(evt.payload.to_vec()).unwrap();
            received.push(s);
        }

        assert_eq!(received.len(), 30);
        // Verify every frame is intact (no byte interleaving)
        for s in received {
            assert!(
                s.starts_with("SRC_1_") || s.starts_with("SRC_2_") || s.starts_with("SRC_3_"),
                "Atomic frame corrupted: {}",
                s
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_burst_push_wakes_all_waiting_consumers() {
        // Two consumers are already waiting when a burst of two events is pushed; both
        // must be woken and every queued event must reach exactly one consumer.
        let q = Arc::new(BoundedEventQueue::new(10, BackpressurePolicy::DropNewest));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let queue = Arc::clone(&q);
            handles.push(tokio::spawn(async move {
                tokio::time::timeout(Duration::from_secs(5), queue.recv(1)).await
            }));
        }

        // Let both consumer tasks poll `recv` and register as waiters before pushing.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        q.push(make_event(1, b"first")).unwrap();
        q.push(make_event(1, b"second")).unwrap();

        let mut payloads = Vec::new();
        for handle in handles {
            let timed = handle.await.unwrap();
            let event = timed
                .expect("a waiting consumer was never woken")
                .expect("recv failed unexpectedly");
            payloads.push(event.payload.to_vec());
        }
        payloads.sort();
        assert_eq!(payloads, vec![b"first".to_vec(), b"second".to_vec()]);
        assert!(q.is_empty());
    }

    #[tokio::test]
    async fn test_close_wakes_waiting_receiver_with_channel_closed() {
        let q = Arc::new(BoundedEventQueue::new(1, BackpressurePolicy::DropNewest));
        let receiver_queue = Arc::clone(&q);
        let handle = tokio::spawn(async move { receiver_queue.recv(7).await });
        tokio::task::yield_now().await;

        q.close();
        let result = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("waiting receiver was not woken by close()")
            .unwrap();
        assert!(
            matches!(result, Err(RoutingError::ChannelClosed(7))),
            "expected ChannelClosed, got {result:?}"
        );
    }

    #[test]
    fn test_delayed_route_outside_async_runtime_does_not_panic() {
        // `route` used to call `tokio::spawn` unconditionally for delayed edges, which
        // panics when the caller is not inside a Tokio runtime (sync CLI setup/tests).
        let graph = RoutingGraph::new();
        graph
            .register_endpoint(1, 10, BackpressurePolicy::DropNewest)
            .unwrap();
        graph
            .register_endpoint(2, 10, BackpressurePolicy::DropNewest)
            .unwrap();
        graph
            .add_edge(
                1,
                2,
                Some(FaultConfig {
                    fixed_delay: Duration::from_millis(5),
                    ..FaultConfig::default()
                }),
            )
            .unwrap();

        let dispatched = graph.route(make_event(1, b"delayed")).unwrap();
        assert_eq!(dispatched, 1);
        // Without a runtime the frame is delivered immediately rather than spawned.
        let received = graph.try_recv(2).unwrap().expect("frame delivered");
        assert_eq!(&received.payload[..], b"delayed");
    }

    #[test]
    fn test_backpressure_drop_newest() {
        let q = BoundedEventQueue::new(2, BackpressurePolicy::DropNewest);
        assert!(q.push(make_event(1, b"1")).unwrap());
        assert!(q.push(make_event(1, b"2")).unwrap());
        // Queue full: drop newest
        assert!(!q.push(make_event(1, b"3")).unwrap());

        assert_eq!(&q.try_recv(1).unwrap().unwrap().payload[..], b"1");
        assert_eq!(&q.try_recv(1).unwrap().unwrap().payload[..], b"2");
        assert!(q.try_recv(1).unwrap().is_none());
    }

    #[test]
    fn test_backpressure_drop_oldest() {
        let q = BoundedEventQueue::new(2, BackpressurePolicy::DropOldest);
        assert!(q.push(make_event(1, b"1")).unwrap());
        assert!(q.push(make_event(1, b"2")).unwrap());
        // Queue full: evicts "1", appends "3"
        assert!(q.push(make_event(1, b"3")).unwrap());

        assert_eq!(&q.try_recv(1).unwrap().unwrap().payload[..], b"2");
        assert_eq!(&q.try_recv(1).unwrap().unwrap().payload[..], b"3");
        assert!(q.try_recv(1).unwrap().is_none());
    }
}
